use clap::{Parser, ValueEnum};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
use yysort::{
    Result,
    cards::{Mode, Plan},
    encoding, engine, error,
    keys::{Field, Format, Key},
    resources,
};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum InputFormat {
    Fixed,
    Csv,
}
#[derive(Parser, Debug)]
#[command(
    version,
    about = "External sorting for fixed records and RFC 4180 CSV",
    long_about = "Bounded, stable sorting/merging/copying. DFSORT control-card subset; unsupported statements fail explicitly. See --manual for formats, compatibility and resource limits."
)]
struct Args {
    #[arg(required_unless_present = "manual", value_name = "FILE")]
    inputs: Vec<PathBuf>,
    #[arg(short, long, required_unless_present_any = ["manual","out"])]
    output: Option<PathBuf>,
    /// Bind an OUTFIL DD name to a file: --out DETAIL=detail.dat (repeatable).
    #[arg(long)]
    out: Vec<std::ffi::OsString>,
    #[arg(long, value_enum, default_value = "fixed")]
    format: InputFormat,
    /// Required fixed record length in bytes (no line separators).
    #[arg(long)]
    record_length: Option<usize>,
    #[arg(long, default_value = "UTF-8")]
    encoding: String,
    /// CSV only; fixed outputs retain original bytes.
    #[arg(long)]
    output_encoding: Option<String>,
    /// yyeditor .map files; may be repeated.
    #[arg(long)]
    mapping: Vec<PathBuf>,
    /// Fixed: position:length:CH|TEXT|BI|FI|PD|ZD:A|D. CSV: column:TEXT|NUM:A|D.
    #[arg(short, long)]
    key: Vec<String>,
    /// UTF-8 free-form DFSORT control-card file (maximum 1 MiB).
    #[arg(long)]
    control: Option<PathBuf>,
    /// Merge already sorted inputs; reject out-of-order records.
    #[arg(long, conflicts_with = "copy")]
    merge: bool,
    #[arg(long)]
    copy: bool,
    /// SUM FIELDS=NONE: retain the first record of each equal-key group.
    #[arg(long)]
    unique: bool,
    /// Fixed: position:length:BI|FI|PD|ZD. CSV: column:NUM.
    #[arg(long)]
    sum: Vec<String>,
    /// Preserve one CSV header; all input headers must match.
    #[arg(long)]
    header: bool,
    /// Accept LF record separators in addition to RFC 4180 CRLF.
    #[arg(long)]
    allow_lf: bool,
    /// Worker memory ceiling: Linux virtual address space / Windows committed memory.
    #[arg(long,default_value="512MiB",value_parser=size)]
    memory: u64,
    /// Maximum logical CPUs and sorting worker threads (no background global pool).
    #[arg(long,default_value_t=default_threads(),value_parser=clap::value_parser!(u16).range(1..=256))]
    threads: u16,
    /// Maximum decoded CSV record/fixed record/BUILD result size.
    #[arg(long,default_value="64KiB",value_parser=size)]
    max_record: u64,
    /// Maximum simultaneously live sort run bytes (final staged output excluded).
    #[arg(long,default_value="64GiB",value_parser=size)]
    temp_limit: u64,
    /// Maximum input runs per merge, additionally reduced to fit memory.
    #[arg(long, default_value_t=32, value_parser=clap::value_parser!(u16).range(2..=128))]
    merge_fan_in: u16,
    #[arg(long)]
    temp_dir: Option<PathBuf>,
    /// Cumulative worker CPU seconds; OS enforced (Windows counts user CPU time).
    #[arg(long,value_parser=clap::value_parser!(u64).range(1..))]
    cpu_seconds: Option<u64>,
    #[arg(long)]
    overwrite: bool,
    /// Print JSON processing statistics to stderr.
    #[arg(long)]
    stats: bool,
    /// Print the complete offline manual and supported control-card grammar.
    #[arg(long)]
    manual: bool,
}
fn default_threads() -> u16 {
    std::thread::available_parallelism()
        .map(|n| n.get().min(2) as u16)
        .unwrap_or(1)
}
fn size(s: &str) -> std::result::Result<u64, String> {
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let n = s[..split]
        .parse::<u64>()
        .map_err(|_| "size requires an unsigned integer".to_owned())?;
    let factor = match s[split..].to_ascii_uppercase().as_str() {
        "" | "B" => 1,
        "KIB" => 1024,
        "MIB" => 1024 * 1024,
        "GIB" => 1024 * 1024 * 1024,
        "TIB" => 1024u64.pow(4),
        _ => return Err("use B, KiB, MiB, GiB or TiB".into()),
    };
    n.checked_mul(factor)
        .filter(|n| *n > 0)
        .ok_or_else(|| "invalid/overflowing size".into())
}
fn parse_key(s: &str, csv: bool) -> Result<Key> {
    let t: Vec<_> = s.split(':').collect();
    let expected = if csv { 3 } else { 4 };
    if t.len() != expected {
        return Err(error(
            "key syntax: fixed p:length:format:A|D; CSV column:TEXT|NUM:A|D",
        ));
    }
    let position = t[0].parse()?;
    let length = if csv { 1 } else { t[1].parse()? };
    if position == 0 || length == 0 {
        return Err(error("positions and lengths must be nonzero"));
    }
    let format = Format::parse(t[expected - 2])?;
    if csv && !matches!(format, Format::Text | Format::CH | Format::Number) {
        return Err(error("CSV keys require TEXT, CH or NUM"));
    }
    let descending = match t[expected - 1].to_ascii_uppercase().as_str() {
        "A" => false,
        "D" => true,
        _ => return Err(error("key direction must be A or D")),
    };
    Ok(Key {
        field: Field {
            position,
            length,
            format,
        },
        descending,
    })
}
fn plan(args: &Args, enc: yy_encoding::Encoding) -> Result<Plan> {
    let csv = matches!(args.format, InputFormat::Csv);
    if let Some(path) = &args.control {
        if !args.key.is_empty() || args.merge || args.copy || args.unique || !args.sum.is_empty() {
            return Err(error(
                "--control cannot be combined with --key/--merge/--copy/--unique/--sum",
            ));
        }
        if std::fs::metadata(path)?.len() > 1024 * 1024 {
            return Err(error("control file exceeds 1 MiB"));
        }
        return yysort::cards::parse(
            &std::fs::read_to_string(path)?,
            if csv {
                yy_encoding::Encoding::Utf8
            } else {
                enc
            },
        );
    }
    let mut plan = Plan {
        mode: if args.copy {
            Mode::Copy
        } else if args.merge {
            Mode::Merge
        } else {
            Mode::Sort
        },
        keys: args
            .key
            .iter()
            .map(|s| parse_key(s, csv))
            .collect::<Result<_>>()?,
        ..Default::default()
    };
    if args.unique && !args.sum.is_empty() {
        return Err(error("choose --unique or --sum"));
    }
    if args.unique {
        plan.sums = Some(Vec::new());
    }
    if !args.sum.is_empty() {
        let mut fields = Vec::new();
        for s in &args.sum {
            let key = parse_key(&format!("{s}:A"), csv)?;
            fields.push(key.field);
        }
        plan.sums = Some(fields);
    }
    Ok(plan)
}
fn load_mappings(args: &Args) -> Result<()> {
    for path in &args.mapping {
        if std::fs::metadata(path)?.len() > 16 * 1024 * 1024 {
            return Err(error("mapping file exceeds 16 MiB"));
        }
        let map = yy_encoding::parse_mapping(
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("custom"),
            &std::fs::read_to_string(path)?,
        )
        .map_err(error)?;
        yy_encoding::register_mapping(map).map_err(error)?;
    }
    Ok(())
}
fn worker(args: &Args, scratch: PathBuf, staging: PathBuf) -> Result<()> {
    let guard = resources::apply(args.memory, args.threads as usize, args.cpu_seconds)?;
    load_mappings(args)?;
    let encoding = encoding::parse(&args.encoding)?;
    let output_encoding =
        encoding::parse(args.output_encoding.as_deref().unwrap_or(&args.encoding))?;
    let fixed = match args.format {
        InputFormat::Fixed => Some(
            args.record_length
                .ok_or_else(|| error("--record-length is required for fixed input"))?,
        ),
        InputFormat::Csv => {
            if args.record_length.is_some() {
                return Err(error("--record-length is for fixed files"));
            }
            None
        }
    };
    if args.max_record > 16 * 1024 * 1024 {
        return Err(error("--max-record must not exceed 16 MiB"));
    }
    let stats = engine::run(engine::Options {
        inputs: args.inputs.clone(),
        outputs: yysort::outputs::read_manifest(&staging)?,
        scratch,
        fixed,
        encoding,
        output_encoding,
        header: args.header,
        allow_lf: args.allow_lf,
        max_record: usize::try_from(args.max_record)?,
        memory: args.memory,
        temp_limit: args.temp_limit,
        merge_fan_in: args.merge_fan_in as usize,
        threads: args.threads as usize,
        plan: plan(args, encoding)?,
    })?;
    if args.stats {
        let mut stats = serde_json::to_value(&stats)?;
        stats["resources"] = serde_json::to_value(guard.observed()?)?;
        eprintln!("{}", serde_json::to_string(&stats)?);
    }
    Ok(())
}
fn snapshot(path: &Path) -> Result<(PathBuf, u64, std::time::SystemTime)> {
    let path = std::fs::canonicalize(path)?;
    let metadata = std::fs::metadata(&path)?;
    if !metadata.is_file() {
        return Err(error("inputs must be regular files"));
    }
    Ok((path, metadata.len(), metadata.modified()?))
}
fn supervise(args: &Args) -> Result<i32> {
    if args.memory < 64 * 1024 * 1024 {
        return Err(error(
            "--memory must be at least 64 MiB (worker includes codecs, allocator and thread stacks)",
        ));
    }
    let mut inputs = args
        .inputs
        .iter()
        .map(|p| snapshot(p))
        .collect::<Result<Vec<_>>>()?;
    if let Some(path) = &args.control {
        inputs.push(snapshot(path)?);
    }
    for path in &args.mapping {
        inputs.push(snapshot(path)?);
    }
    let protected: Vec<_> = inputs.iter().map(|(p, _, _)| p.clone()).collect();
    let mut bindings = Vec::new();
    if let Some(output) = &args.output {
        bindings.push(("SORTOUT".to_owned(), output.clone()));
    }
    for binding in &args.out {
        bindings.push(yysort::outputs::binding(binding)?);
    }
    let transaction = yysort::outputs::Transaction::prepare(bindings, &protected, args.overwrite)?;
    let scratch = tempfile::Builder::new().prefix("yysort-").tempdir_in(
        args.temp_dir
            .as_deref()
            .unwrap_or(transaction.first_parent()),
    )?;
    let manifest = scratch.path().join("outputs.json");
    yysort::outputs::write_manifest(&manifest, transaction.stages())?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(std::env::args_os().skip(1))
        .env("YYSORT_WORKER_DIR", scratch.path())
        .env("YYSORT_STAGED_OUTPUT", &manifest)
        .env("MALLOC_ARENA_MAX", "2")
        .stdin(Stdio::null());
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(command.spawn()?);
    let status = loop {
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        if resources::check().is_err() {
            let _ = child.0.kill();
            let _ = child.0.wait();
            return Ok(130);
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    if !status.success() {
        return Ok(status.code().filter(|n| *n != 0).unwrap_or(2));
    }
    resources::check()?;
    for original in inputs {
        if snapshot(&original.0)? != original {
            return Err(error(
                "input changed during processing; output not published",
            ));
        }
    }
    transaction.publish()?;
    Ok(0)
}
fn main() {
    let args = Args::parse();
    if args.manual {
        print!("{}", include_str!("../help/help.md"));
        return;
    }
    let result = (|| -> Result<i32> {
        resources::install_interrupt()?;
        match (
            std::env::var_os("YYSORT_WORKER_DIR"),
            std::env::var_os("YYSORT_STAGED_OUTPUT"),
        ) {
            (Some(dir), Some(output)) => {
                worker(&args, dir.into(), output.into())?;
                Ok(0)
            }
            (None, None) => supervise(&args),
            _ => Err(error("incomplete internal worker environment")),
        }
    })();
    match result {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("yysort: {e}");
            std::process::exit(
                if resources::CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
                    130
                } else {
                    2
                },
            );
        }
    }
}
