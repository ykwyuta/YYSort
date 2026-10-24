use crate::{
    Result,
    cards::Outfil,
    engine::{self, Options},
    error,
    records::{Output, Record},
    resources,
};
use std::{collections::BTreeMap, fs::File, io::BufWriter};
struct FileOutput {
    name: String,
    writer: Output<BufWriter<File>>,
    sync: File,
    count: u64,
}
struct Group {
    plan: Outfil,
    targets: Vec<usize>,
    accepted: u64,
}
pub(crate) struct Router {
    files: Vec<FileOutput>,
    groups: Vec<Group>,
    main: Option<usize>,
    number: u64,
    selected: Vec<bool>,
}
impl Router {
    pub fn new(options: &Options) -> Result<Self> {
        let mut files = Vec::new();
        let mut indices = BTreeMap::new();
        for (name, path) in &options.outputs {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(path)?;
            let sync = file.try_clone()?;
            indices.insert(name.clone(), files.len());
            files.push(FileOutput {
                name: name.clone(),
                writer: Output::new(
                    BufWriter::with_capacity(65536, file),
                    options.output_encoding,
                    options.fixed.is_none(),
                ),
                sync,
                count: 0,
            });
        }
        let mut groups = Vec::new();
        let mut used = std::collections::HashSet::new();
        for plan in &options.plan.outfils {
            let mut targets = Vec::new();
            for name in &plan.names {
                if !used.insert(name) {
                    return Err(error(format!("OUTFIL DD name used more than once: {name}")));
                }
                targets.push(*indices.get(name).ok_or_else(|| {
                    error(format!(
                        "OUTFIL {name} needs --out {name}=path (SORTOUT may use -o)"
                    ))
                })?);
            }
            groups.push(Group {
                plan: plan.clone(),
                targets,
                accepted: 0,
            });
        }
        for name in indices.keys() {
            if name != "SORTOUT" && !used.contains(name) {
                return Err(error(format!("output binding has no OUTFIL group: {name}")));
            }
        }
        let main = if used.contains(&"SORTOUT".to_owned()) {
            None
        } else {
            indices.get("SORTOUT").copied()
        };
        Ok(Self {
            files,
            selected: vec![false; groups.len()],
            groups,
            main,
            number: 0,
        })
    }
    pub fn header(&mut self, header: &Record) -> Result<()> {
        for file in &mut self.files {
            file.writer.record(header)?;
        }
        Ok(())
    }
    pub fn record(&mut self, record: &Record, options: &Options) -> Result<()> {
        resources::check()?;
        self.number = self
            .number
            .checked_add(1)
            .ok_or_else(|| error("OUTFIL record count overflow"))?;
        if let Some(i) = self.main {
            self.files[i].writer.record(record)?;
            self.files[i].count += 1;
        }
        self.selected.fill(false);
        let mut matched = false;
        for (i, group) in self.groups.iter().enumerate() {
            if !group.plan.save && accepts(group, self.number, record, options)? {
                self.selected[i] = true;
                matched = true;
            }
        }
        for (i, group) in self.groups.iter().enumerate() {
            if group.plan.save && !matched && accepts(group, self.number, record, options)? {
                self.selected[i] = true;
            }
        }
        for (group, selected) in self.groups.iter_mut().zip(self.selected.iter().copied()) {
            if !selected {
                continue;
            }
            group.accepted = group
                .accepted
                .checked_add(1)
                .ok_or_else(|| error("OUTFIL accepted count overflow"))?;
            let built = if let Some(build) = &group.plan.build {
                let mut built = record.clone();
                engine::transform(&mut built, build, options.max_record)?;
                Some(built)
            } else {
                None
            };
            let record = built.as_ref().unwrap_or(record);
            if let Some((block, contiguous)) = group.plan.split {
                let turn = (group.accepted - 1) / block;
                let target = if contiguous {
                    turn.min(group.targets.len() as u64 - 1) as usize
                } else {
                    (turn % group.targets.len() as u64) as usize
                };
                let file = &mut self.files[group.targets[target]];
                file.writer.record(record)?;
                file.count += 1;
            } else {
                for i in &group.targets {
                    self.files[*i].writer.record(record)?;
                    self.files[*i].count += 1;
                }
            }
        }
        Ok(())
    }
    pub fn finish(self) -> Result<BTreeMap<String, u64>> {
        let mut counts = BTreeMap::new();
        for file in self.files {
            file.writer.finish()?;
            file.sync.sync_all()?;
            counts.insert(file.name, file.count);
        }
        Ok(counts)
    }
}
fn accepts(group: &Group, number: u64, record: &Record, options: &Options) -> Result<bool> {
    let p = &group.plan;
    if number < p.start
        || p.end.is_some_and(|end| number > end)
        || p.accept.is_some_and(|n| group.accepted >= n)
    {
        return Ok(false);
    }
    if (number - p.start) % p.sample.0 >= p.sample.1 {
        return Ok(false);
    }
    if let Some(condition) = &p.include
        && !engine::condition(condition, record, options)?
    {
        return Ok(false);
    }
    if let Some(condition) = &p.omit
        && engine::condition(condition, record, options)?
    {
        return Ok(false);
    }
    Ok(true)
}
