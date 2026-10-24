//! Bounded run generation, online compaction and multi-pass heap merge.
use crate::{
    Result,
    cards::{Expr, Mode, Piece, Plan, Transform},
    error,
    keys::{self, Value},
    records::{self, Reader, Record},
    resources,
};
use rayon::prelude::*;
use serde::Serialize;
use std::{
    cell::Cell,
    cmp::Ordering,
    collections::BinaryHeap,
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    rc::Rc,
};
use tempfile::NamedTempFile;
use yy_encoding::Encoding;

pub struct Options {
    pub inputs: Vec<PathBuf>,
    pub outputs: std::collections::BTreeMap<String, PathBuf>,
    pub scratch: PathBuf,
    pub fixed: Option<usize>,
    pub encoding: Encoding,
    pub output_encoding: Encoding,
    pub header: bool,
    pub allow_lf: bool,
    pub max_record: usize,
    pub memory: u64,
    pub temp_limit: u64,
    pub merge_fan_in: usize,
    pub threads: usize,
    pub plan: Plan,
}
#[derive(Default, Serialize)]
pub struct Stats {
    pub input_records: u64,
    pub selected_records: u64,
    pub output_records: u64,
    pub output_files: std::collections::BTreeMap<String, u64>,
    pub initial_runs: u64,
    pub merge_operations: u64,
    pub peak_temp_bytes: u64,
    pub peak_accounted_chunk_bytes: usize,
    pub fan_in: usize,
    pub elapsed_seconds: f64,
}
#[derive(Clone, Debug)]
struct Entry {
    record: Record,
    values: Vec<(Value, bool)>,
    sequence: u64,
}
impl Entry {
    fn size(&self) -> usize {
        self.record.size()
            + self.values.capacity() * std::mem::size_of::<(Value, bool)>()
            + self.values.iter().map(|(v, _)| v.size()).sum::<usize>()
            + std::mem::size_of::<Self>()
    }
    fn keys_cmp(&self, other: &Self) -> Ordering {
        for ((a, desc), (b, _)) in self.values.iter().zip(&other.values) {
            let cmp = a.cmp(b);
            if cmp != Ordering::Equal {
                return if *desc { cmp.reverse() } else { cmp };
            }
        }
        Ordering::Equal
    }
    fn cmp(&self, other: &Self) -> Ordering {
        self.keys_cmp(other)
            .then_with(|| self.sequence.cmp(&other.sequence))
    }
}
struct Head {
    entry: Entry,
    source: usize,
}
impl Ord for Head {
    fn cmp(&self, other: &Self) -> Ordering {
        other.entry.cmp(&self.entry)
    }
}
impl PartialOrd for Head {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for Head {
    fn eq(&self, other: &Self) -> bool {
        self.entry.cmp(&other.entry) == Ordering::Equal
    }
}
impl Eq for Head {}
struct Disk {
    used: Cell<u64>,
    peak: Cell<u64>,
    limit: u64,
}
struct Run {
    file: NamedTempFile,
    size: u64,
    disk: Rc<Disk>,
}
impl Drop for Run {
    fn drop(&mut self) {
        self.disk.used.set(self.disk.used.get() - self.size);
    }
}
struct Writer<'a> {
    writer: BufWriter<&'a mut File>,
    size: u64,
    disk: Rc<Disk>,
}
impl Write for Writer<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let projected = self
            .disk
            .used
            .get()
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| std::io::Error::other("temporary storage counter overflow"))?;
        if projected > self.disk.limit {
            return Err(std::io::Error::other("--temp-limit exceeded"));
        }
        self.writer.write_all(bytes)?;
        self.disk.used.set(projected);
        self.size += bytes.len() as u64;
        self.disk.peak.set(self.disk.peak.get().max(projected));
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}
fn build_run<F>(scratch: &Path, disk: &Rc<Disk>, f: F) -> Result<Run>
where
    F: FnOnce(&mut Writer<'_>) -> Result<()>,
{
    let mut file = NamedTempFile::new_in(scratch)?;
    let (result, size) = {
        let mut writer = Writer {
            writer: BufWriter::with_capacity(65536, file.as_file_mut()),
            size: 0,
            disk: disk.clone(),
        };
        let result = f(&mut writer).and_then(|_| writer.flush().map_err(Into::into));
        (result, writer.size)
    };
    if let Err(e) = result {
        disk.used.set(disk.used.get() - size);
        return Err(e);
    }
    Ok(Run {
        file,
        size,
        disk: disk.clone(),
    })
}
fn put(writer: &mut impl Write, entry: &Entry) -> Result<()> {
    writer.write_all(&entry.sequence.to_le_bytes())?;
    writer.write_all(&u32::try_from(entry.record.data.len())?.to_le_bytes())?;
    writer.write_all(&u32::try_from(entry.record.fields.len())?.to_le_bytes())?;
    writer.write_all(&entry.record.data)?;
    for field in &entry.record.fields {
        writer.write_all(&u32::try_from(field.start)?.to_le_bytes())?;
        writer.write_all(&u32::try_from(field.end)?.to_le_bytes())?;
    }
    Ok(())
}
fn get(reader: &mut impl Read, options: &Options) -> Result<Option<Entry>> {
    resources::check()?;
    let mut seq = [0; 8];
    let n = reader.read(&mut seq[..1])?;
    if n == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut seq[1..])?;
    let mut word = [0; 4];
    reader.read_exact(&mut word)?;
    let length = u32::from_le_bytes(word) as usize;
    reader.read_exact(&mut word)?;
    let count = u32::from_le_bytes(word) as usize;
    if length > options.max_record || count > options.max_record + 1 {
        return Err(error("invalid temporary record length"));
    }
    let mut data = vec![0; length];
    reader.read_exact(&mut data)?;
    let mut fields = Vec::with_capacity(count);
    let mut previous = 0;
    for _ in 0..count {
        reader.read_exact(&mut word)?;
        let start = u32::from_le_bytes(word) as usize;
        reader.read_exact(&mut word)?;
        let end = u32::from_le_bytes(word) as usize;
        if start != previous || end < start || end > length {
            return Err(error("invalid temporary CSV field"));
        }
        fields.push(start..end);
        previous = end;
    }
    Ok(Some(entry(
        Record { data, fields },
        u64::from_le_bytes(seq),
        options,
    )?))
}
fn entry(record: Record, sequence: u64, options: &Options) -> Result<Entry> {
    let csv = options.fixed.is_none();
    let enc = if csv {
        Encoding::Utf8
    } else {
        options.encoding
    };
    for field in options.plan.sums.iter().flatten() {
        keys::decode(
            record.value(field.position, field.length, csv)?,
            field.format,
            enc,
        )?;
    }
    let values = options
        .plan
        .keys
        .iter()
        .map(|k| {
            Ok((
                keys::decode(
                    record.value(k.field.position, k.field.length, csv)?,
                    k.field.format,
                    enc,
                )?,
                k.descending,
            ))
        })
        .collect::<Result<_>>()?;
    Ok(Entry {
        record,
        values,
        sequence,
    })
}
pub(crate) fn condition(expr: &Expr, record: &Record, options: &Options) -> Result<bool> {
    match expr {
        Expr::Constant(value) => Ok(*value),
        Expr::Compare(field, op, value) => {
            let csv = options.fixed.is_none();
            let actual = keys::decode(
                record.value(field.position, field.length, csv)?,
                field.format,
                if csv {
                    Encoding::Utf8
                } else {
                    options.encoding
                },
            )?;
            Ok(op.matches(actual.cmp(value)))
        }
        Expr::CompareFields(left, op, right) => {
            let csv = options.fixed.is_none();
            let enc = if csv {
                Encoding::Utf8
            } else {
                options.encoding
            };
            let left = keys::decode(
                record.value(left.position, left.length, csv)?,
                left.format,
                enc,
            )?;
            let right = keys::decode(
                record.value(right.position, right.length, csv)?,
                right.format,
                enc,
            )?;
            Ok(op.matches(left.cmp(&right)))
        }
        Expr::And(a, b) => Ok(condition(a, record, options)? && condition(b, record, options)?),
        Expr::Or(a, b) => Ok(condition(a, record, options)? || condition(b, record, options)?),
    }
}
pub(crate) fn transform(record: &mut Record, t: &Transform, max: usize) -> Result<()> {
    let length = t.output_length(record.data.len())?;
    if length > max {
        return Err(error("reformat result exceeds --max-record"));
    }
    let mut data = if t.overlay {
        std::mem::take(&mut record.data)
    } else {
        Vec::with_capacity(length)
    };
    let mut cursor = 0;
    for piece in &t.pieces {
        if let Piece::Position(position) = piece {
            cursor = *position - 1;
            continue;
        }
        // Later OVERLAY items read the current reformatted record. Copy within a single item
        // uses memmove semantics so even overlapping source/destination ranges are preserved.
        match piece {
            Piece::Field(p, n) if t.overlay => {
                let start = *p - 1;
                let end = start
                    .checked_add(*n)
                    .ok_or_else(|| error("field offset overflow"))?;
                if end > data.len() {
                    return Err(error("OVERLAY source field exceeds current record length"));
                }
                if cursor > data.len() {
                    data.resize(cursor, t.padding[0]);
                }
                data.resize(data.len().max(cursor + n), 0);
                data.copy_within(start..end, cursor);
                cursor += n;
            }
            Piece::Field(p, n) => {
                let bytes = record.value(*p, *n, false)?;
                if cursor > data.len() {
                    data.resize(cursor, t.padding[0]);
                }
                data.extend_from_slice(bytes);
                cursor += n;
            }
            Piece::Literal(bytes) => {
                if cursor > data.len() {
                    data.resize(cursor, t.padding[0]);
                }
                data.resize(data.len().max(cursor + bytes.len()), 0);
                data[cursor..cursor + bytes.len()].copy_from_slice(bytes);
                cursor += bytes.len();
            }
            Piece::Blank(n) => {
                if cursor > data.len() {
                    data.resize(cursor, t.padding[0]);
                }
                data.resize(data.len().max(cursor + n), t.padding[0]);
                data[cursor..cursor + n].fill(t.padding[0]);
                cursor += n;
            }
            Piece::Position(_) => unreachable!(),
        }
    }
    record.data = data;
    Ok(())
}
fn selected(record: &Record, options: &Options) -> Result<bool> {
    if let Some(expr) = &options.plan.include
        && !condition(expr, record, options)?
    {
        return Ok(false);
    }
    if let Some(expr) = &options.plan.omit
        && condition(expr, record, options)?
    {
        return Ok(false);
    }
    Ok(true)
}
fn merge<F>(runs: &[Run], options: &Options, mut emit: F) -> Result<()>
where
    F: FnMut(&Entry) -> Result<()>,
{
    let mut readers: Vec<_> = runs
        .iter()
        .map(|r| File::open(r.file.path()).map(|f| BufReader::with_capacity(8192, f)))
        .collect::<std::io::Result<_>>()?;
    let mut heap = BinaryHeap::new();
    for (i, reader) in readers.iter_mut().enumerate() {
        if let Some(entry) = get(reader, options)? {
            heap.push(Head { entry, source: i });
        }
    }
    while let Some(head) = heap.pop() {
        emit(&head.entry)?;
        if let Some(entry) = get(&mut readers[head.source], options)? {
            heap.push(Head {
                entry,
                source: head.source,
            });
        }
    }
    Ok(())
}
fn compact(
    mut run: Run,
    levels: &mut Vec<Vec<Run>>,
    options: &Options,
    disk: &Rc<Disk>,
    stats: &mut Stats,
) -> Result<()> {
    let mut level = 0;
    loop {
        if level == levels.len() {
            levels.push(Vec::new());
        }
        levels[level].push(run);
        if levels[level].len() < stats.fan_in {
            return Ok(());
        }
        let source = std::mem::take(&mut levels[level]);
        run = build_run(&options.scratch, disk, |writer| {
            merge(&source, options, |entry| put(writer, entry))
        })?;
        stats.merge_operations += 1;
        drop(source);
        level += 1;
    }
}
fn flush_chunk(
    chunk: &mut Vec<Entry>,
    levels: &mut Vec<Vec<Run>>,
    options: &Options,
    disk: &Rc<Disk>,
    pool: &rayon::ThreadPool,
    stats: &mut Stats,
) -> Result<()> {
    resources::check()?;
    pool.install(|| chunk.par_sort_unstable_by(Entry::cmp));
    resources::check()?;
    let run = build_run(&options.scratch, disk, |writer| {
        for e in chunk.iter() {
            resources::check()?;
            put(writer, e)?;
        }
        Ok(())
    })?;
    chunk.clear();
    chunk.shrink_to_fit();
    stats.initial_runs += 1;
    compact(run, levels, options, disk, stats)
}
pub fn run(options: Options) -> Result<Stats> {
    validate(&options)?;
    let started = std::time::Instant::now();
    let budget = usize::try_from(options.memory / 4)?;
    let record_bound = if let Some(transform) = &options.plan.inrec {
        transform.output_length(options.fixed.unwrap_or(options.max_record))?
    } else {
        options.fixed.unwrap_or(options.max_record)
    };
    if record_bound > options.max_record {
        return Err(error("INREC BUILD exceeds --max-record"));
    }
    // Include CSV field ranges, decoded key expansion and repeated/overlapping keys.
    // Reserve three extra heads for incoming records, group accumulation and decode scratch.
    let key_bound = options.plan.keys.iter().try_fold(0usize, |total, key| {
        let length = if options.fixed.is_some() {
            key.field.length
        } else {
            options.max_record
        };
        total
            .checked_add(length.saturating_mul(16).saturating_add(512))
            .ok_or_else(|| error("sort key memory bound overflow"))
    })?;
    let worst = record_bound
        .saturating_mul(if options.fixed.is_some() { 4 } else { 40 })
        .saturating_add(key_bound)
        .max(65536);
    let heads = budget / worst;
    if heads < 5 {
        return Err(error(
            "memory limit too small for --max-record and two merge heads; increase --memory or reduce --max-record",
        ));
    }
    let fan_in = (heads - 3).clamp(2, options.merge_fan_in);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.threads)
        .stack_size(2 * 1024 * 1024)
        .build()?;
    let disk = Rc::new(Disk {
        used: Cell::new(0),
        peak: Cell::new(0),
        limit: options.temp_limit,
    });
    let mut stats = Stats {
        fan_in,
        ..Default::default()
    };
    let mut levels: Vec<Vec<Run>> = Vec::new();
    let mut chunk = Vec::new();
    let mut accounted = 0;
    let mut header: Option<Record> = None;
    let mut output = crate::routing::Router::new(&options)?;
    for path in &options.inputs {
        let mut reader = Reader::open(
            path,
            options.fixed,
            options.encoding,
            options.max_record,
            options.allow_lf,
        )?;
        if options.header {
            let next = reader
                .next_record()?
                .ok_or_else(|| error("CSV header requested but input is empty"))?;
            if let Some(header) = &header {
                if next.data != header.data || next.fields != header.fields {
                    return Err(error("CSV input headers differ"));
                }
            } else {
                output.header(&next)?;
                header = Some(next);
            }
        }
        let mut previous: Option<Entry> = None;
        let mut sorted_run = if options.plan.mode == Mode::Merge {
            Some(NamedTempFile::new_in(&options.scratch)?)
        } else {
            None
        };
        let mut run_writer = sorted_run.as_mut().map(|f| Writer {
            writer: BufWriter::with_capacity(65536, f.as_file_mut()),
            size: 0,
            disk: disk.clone(),
        });
        loop {
            if options
                .plan
                .take
                .is_some_and(|n| stats.selected_records >= n)
            {
                break;
            }
            let Some(mut record) = reader.next_record()? else {
                break;
            };
            stats.input_records = stats
                .input_records
                .checked_add(1)
                .ok_or_else(|| error("record counter overflow"))?;
            if stats.input_records <= options.plan.skip {
                continue;
            }
            if !selected(&record, &options)? {
                continue;
            }
            if let Some(t) = &options.plan.inrec {
                transform(&mut record, t, options.max_record)?;
            }
            stats.selected_records += 1;
            let mut e = entry(record, stats.input_records, &options)?;
            if options.plan.mode == Mode::Copy {
                if let Some(t) = &options.plan.outrec {
                    transform(&mut e.record, t, options.max_record)?;
                }
                output.record(&e.record, &options)?;
                stats.output_records += 1;
                continue;
            }
            if let Some(writer) = &mut run_writer {
                if previous
                    .as_ref()
                    .is_some_and(|p| p.keys_cmp(&e) == Ordering::Greater)
                {
                    return Err(error(format!(
                        "MERGE input is not sorted: {} record {}",
                        path.display(),
                        stats.input_records
                    )));
                }
                put(writer, &e)?;
                previous = Some(e);
                continue;
            }
            let size = e
                .size()
                .checked_mul(2)
                .ok_or_else(|| error("record memory counter overflow"))?;
            if size > budget / 4 {
                return Err(error("record and keys exceed bounded chunk capacity"));
            }
            if accounted + size > budget && !chunk.is_empty() {
                flush_chunk(&mut chunk, &mut levels, &options, &disk, &pool, &mut stats)?;
                accounted = 0;
            }
            accounted += size;
            stats.peak_accounted_chunk_bytes = stats.peak_accounted_chunk_bytes.max(accounted);
            chunk.push(e);
        }
        let run_size = if let Some(mut writer) = run_writer.take() {
            writer.flush()?;
            Some(writer.size)
        } else {
            None
        };
        drop(run_writer);
        if let Some(size) = run_size {
            let run = Run {
                file: sorted_run.take().unwrap(),
                size,
                disk: disk.clone(),
            };
            stats.initial_runs += 1;
            compact(run, &mut levels, &options, &disk, &mut stats)?;
        }
        if options
            .plan
            .take
            .is_some_and(|n| stats.selected_records >= n)
        {
            break;
        }
    }
    if !chunk.is_empty() {
        flush_chunk(&mut chunk, &mut levels, &options, &disk, &pool, &mut stats)?;
    }
    // At most fan_in-1 runs per level; flattening is logarithmically bounded.
    let mut runs: Vec<_> = levels.into_iter().flatten().collect();
    while runs.len() > fan_in {
        let source: Vec<_> = runs.drain(..fan_in).collect();
        let run = build_run(&options.scratch, &disk, |writer| {
            merge(&source, &options, |entry| put(writer, entry))
        })?;
        stats.merge_operations += 1;
        drop(source);
        runs.push(run);
    }
    let mut group: Option<Entry> = None;
    merge(&runs, &options, |entry| {
        if options.plan.sums.is_some() {
            if let Some(current) = &mut group
                && current.keys_cmp(entry) == Ordering::Equal
            {
                sum(current, entry, &options)?;
                return Ok(());
            }
            if let Some(mut old) = group.replace(entry.clone()) {
                emit(&mut old, &options, &mut output)?;
                stats.output_records += 1;
            }
        } else {
            let mut entry = entry.clone();
            emit(&mut entry, &options, &mut output)?;
            stats.output_records += 1;
        }
        Ok(())
    })?;
    if let Some(mut last) = group {
        emit(&mut last, &options, &mut output)?;
        stats.output_records += 1;
    }
    stats.output_files = output.finish()?;
    stats.peak_temp_bytes = disk.peak.get();
    stats.elapsed_seconds = started.elapsed().as_secs_f64();
    Ok(stats)
}
fn emit(entry: &mut Entry, options: &Options, output: &mut crate::routing::Router) -> Result<()> {
    resources::check()?;
    if let Some(t) = &options.plan.outrec {
        transform(&mut entry.record, t, options.max_record)?;
    }
    output.record(&entry.record, options)
}
fn sum(a: &mut Entry, b: &Entry, options: &Options) -> Result<()> {
    let csv = options.fixed.is_none();
    let enc = if csv {
        Encoding::Utf8
    } else {
        options.encoding
    };
    let mut updates = Vec::new();
    for f in options.plan.sums.as_ref().unwrap() {
        let Value::Number(mut n) =
            keys::decode(a.record.value(f.position, f.length, csv)?, f.format, enc)?
        else {
            return Err(error("SUM field is not numeric"));
        };
        let Value::Number(other) =
            keys::decode(b.record.value(f.position, f.length, csv)?, f.format, enc)?
        else {
            return Err(error("SUM field is not numeric"));
        };
        n.add(&other);
        let bytes = keys::encode_number(&n, f.format, f.length, enc)?;
        if csv {
            updates.push((f.position, bytes));
        } else {
            let start = f.position - 1;
            a.record.data[start..start + f.length].copy_from_slice(&bytes);
        }
    }
    if csv {
        records::replace_fields(&mut a.record, &updates)?;
        if a.record.data.len() > options.max_record {
            return Err(error("SUM output exceeds --max-record"));
        }
    }
    Ok(())
}
fn validate(o: &Options) -> Result<()> {
    if o.memory < 64 * 1024 * 1024
        || !(2..=128).contains(&o.merge_fan_in)
        || o.threads == 0
        || o.max_record == 0
        || o.max_record > 16 * 1024 * 1024
    {
        return Err(error("invalid memory, record, CPU or merge fan-in limit"));
    }
    if o.inputs.is_empty() {
        return Err(error("at least one input is required"));
    }
    if o.fixed.is_some() && o.encoding != o.output_encoding {
        return Err(error(
            "fixed-length output preserves bytes; different output encoding is not supported",
        ));
    }
    if o.fixed.is_some() && o.header {
        return Err(error("--header is for CSV only"));
    }
    if o.fixed.is_none() && (o.plan.inrec.is_some() || o.plan.outrec.is_some()) {
        return Err(error(
            "byte BUILD transformations require fixed-length input",
        ));
    }
    if o.plan.mode != Mode::Copy && o.plan.keys.is_empty() {
        return Err(error("at least one sort key is required"));
    }
    if o.plan.mode == Mode::Copy && o.plan.sums.is_some() {
        return Err(error("SUM is not supported with COPY"));
    }
    if o.plan.mode == Mode::Copy && !o.plan.keys.is_empty() {
        return Err(error("COPY does not accept sort keys"));
    }
    for field in o
        .plan
        .keys
        .iter()
        .map(|k| &k.field)
        .chain(o.plan.sums.iter().flatten())
    {
        if field.position == 0
            || field.length == 0
            || field.position.checked_add(field.length).is_none()
        {
            return Err(error("invalid field position/length"));
        }
        if o.fixed.is_none()
            && (field.length != 1
                || !matches!(
                    field.format,
                    keys::Format::CH | keys::Format::Text | keys::Format::Number
                ))
        {
            return Err(error("CSV control fields require column,1,TEXT|CH|NUM"));
        }
    }
    fn check_condition_field(f: &keys::Field, csv: bool) -> Result<()> {
        if f.position == 0
            || f.length == 0
            || f.position.checked_add(f.length).is_none()
            || csv
                && (f.length != 1
                    || !matches!(
                        f.format,
                        keys::Format::CH | keys::Format::Text | keys::Format::Number
                    ))
        {
            return Err(error("invalid condition field for input format"));
        }
        Ok(())
    }
    fn check_condition(expr: &Expr, csv: bool) -> Result<()> {
        match expr {
            Expr::Constant(_) => {}
            Expr::Compare(f, _, value) => {
                check_condition_field(f, csv)?;
                if !csv
                    && f.format == keys::Format::CH
                    && matches!(value,Value::Bytes(bytes) if bytes.len()!=f.length)
                {
                    return Err(error(
                        "fixed CH condition literals must match field length; implicit padding is unsupported",
                    ));
                }
            }
            Expr::CompareFields(left, _, right) => {
                check_condition_field(left, csv)?;
                check_condition_field(right, csv)?;
            }
            Expr::And(a, b) | Expr::Or(a, b) => {
                check_condition(a, csv)?;
                check_condition(b, csv)?;
            }
        }
        Ok(())
    }
    if o.outputs.is_empty() || o.outputs.len() > 64 {
        return Err(error("between 1 and 64 output files required"));
    }
    if o.plan.outfils.len() > 64 {
        return Err(error("maximum 64 OUTFIL groups"));
    }
    let mut reformatted = o.fixed.unwrap_or(o.max_record);
    for t in [&o.plan.inrec, &o.plan.outrec].into_iter().flatten() {
        reformatted = t.output_length(reformatted)?;
        if reformatted > o.max_record {
            return Err(error("reformat result exceeds --max-record"));
        }
    }
    for group in &o.plan.outfils {
        if group.names.is_empty()
            || group.names.len() > 64
            || group.start == 0
            || group.end.is_some_and(|end| end < group.start)
            || group.sample.0 == 0
            || group.sample.1 == 0
            || group.sample.1 > group.sample.0
            || group.accept == Some(0)
            || group.split.is_some_and(|(block, _)| block == 0)
            || group.include.is_some() && group.omit.is_some()
            || group.save && (group.include.is_some() || group.omit.is_some())
        {
            return Err(error("invalid OUTFIL parameters"));
        }
        if o.fixed.is_none() && group.build.is_some() {
            return Err(error("OUTFIL BUILD requires fixed-length input"));
        }
        if let Some(t) = &group.build
            && t.output_length(reformatted)? > o.max_record
        {
            return Err(error("OUTFIL reformat result exceeds --max-record"));
        }
        for expr in [&group.include, &group.omit].into_iter().flatten() {
            check_condition(expr, o.fixed.is_none())?;
        }
    }
    for expr in [&o.plan.include, &o.plan.omit].into_iter().flatten() {
        check_condition(expr, o.fixed.is_none())?;
    }
    if let Some(fields) = &o.plan.sums {
        for (i, a) in fields.iter().enumerate() {
            for b in &fields[..i] {
                if if o.fixed.is_none() {
                    a.position == b.position
                } else {
                    a.position < b.position.saturating_add(b.length)
                        && b.position < a.position.saturating_add(a.length)
                } {
                    return Err(error("SUM fields must not overlap each other"));
                }
            }
        }
    }
    for f in o.plan.sums.iter().flatten() {
        if !matches!(
            f.format,
            keys::Format::BI
                | keys::Format::FI
                | keys::Format::PD
                | keys::Format::ZD
                | keys::Format::Number
        ) {
            return Err(error("unsupported SUM field type"));
        }
        if o.fixed.is_some() && f.format == keys::Format::Number {
            return Err(error("fixed SUM uses BI/FI/PD/ZD; CSV SUM uses NUM"));
        }
        if o.fixed.is_none() && f.format != keys::Format::Number {
            return Err(error("CSV SUM requires NUM fields"));
        }
        if o.plan.keys.iter().any(|k| {
            if o.fixed.is_none() {
                k.field.position == f.position
            } else {
                let a = f.position - 1;
                let b = k.field.position - 1;
                a < b.saturating_add(k.field.length) && b < a.saturating_add(f.length)
            }
        }) {
            return Err(error("SUM fields must not overlap sort keys"));
        }
    }
    Ok(())
}
