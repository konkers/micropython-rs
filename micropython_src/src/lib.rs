use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    env,
    fmt::Write as FmtWrite,
    fs::{self, File},
    io::Write,
    num::Wrapping,
    path::{Path, PathBuf},
};

use handlebars::Handlebars;
use path_slash::{PathBufExt, PathExt};
use regex::Regex;
use serde::Serialize;

struct Data {
    qstr_ident_translations: HashMap<char, String>,
    static_qstrs: Vec<String>,
    unsorted_qstrs: Vec<String>,
}

impl Data {
    fn new() -> Self {
        let qstr_ident_translations =
            json5::from_str(include_str!("../data/qstr_ident_translations.json5"))
                .unwrap_or_else(|e| panic!("can't parse qstr_ident_translations.json5: {e}"));

        let static_qstrs = json5::from_str(include_str!("../data/static_qstrs.json5"))
            .unwrap_or_else(|e| panic!("can't parse static_qstrs.json5: {e}"));

        let unsorted_qstrs = json5::from_str(include_str!("../data/unsorted_qstrs.json5"))
            .unwrap_or_else(|e| panic!("can't parse unsorted_qstrs.json5: {e}"));
        Self {
            qstr_ident_translations,
            static_qstrs,
            unsorted_qstrs,
        }
    }
}

pub enum BytesIn {
    One,
    Two,
}

impl BytesIn {
    fn mask(&self) -> u32 {
        match self {
            Self::One => 0xff,
            Self::Two => 0xffff,
        }
    }
}

impl Default for BytesIn {
    fn default() -> Self {
        Self::Two
    }
}

#[derive(Default)]
pub struct Config {
    pub bytes_in_hash: BytesIn,
    pub bytes_in_string: BytesIn,
}

pub struct Build {
    out_dir: PathBuf,
    lib_dir: PathBuf,
    include_dir: PathBuf,
    source_dir: PathBuf,
    source_files: Vec<PathBuf>,
    header_files: Vec<PathBuf>,
    data: Data,
    config: Config,
    // target: String,
    //host: String,
}

#[derive(Debug, Serialize)]
struct QStr {
    pub pool: u8,
    pub val: String,
    pub ident: String,
    pub hash: u32,
    pub val_len: usize,
    pub source: String,
}

impl QStr {
    fn new(config: &Config, data: &Data, val: &str, pool: u8, source: String) -> Self {
        Self {
            pool,
            val: Self::escape_string(val),
            ident: Self::ident(data, val),
            hash: Self::hash(val.as_bytes(), &config.bytes_in_hash),
            val_len: val.len(),
            source,
        }
    }

    fn hash(data: &[u8], bytes_in_hash: &BytesIn) -> u32 {
        let mut hash = Wrapping(5381u32);
        for b in data {
            hash = (hash * Wrapping(33)) ^ Wrapping(*b as u32);
        }
        let hash = hash.0 & bytes_in_hash.mask();

        // A hash of 0 indicates "hash not computed" so force any valid 0 hashes
        // to be 1 instead.
        if hash == 0 {
            1
        } else {
            hash
        }
    }

    fn ident(data: &Data, val: &str) -> String {
        let mut s = "MP_QSTR_".to_string();
        for c in val.chars() {
            if let Some(replacement) = data.qstr_ident_translations.get(&c) {
                s.push_str(&format!("_{replacement}_"));
            } else {
                s.push(c);
            }
        }

        s
    }

    fn escape_string(val: &str) -> String {
        if val.chars().all(|c| !c.is_ascii_control()) {
            return val.to_string();
        }

        if val.chars().any(|c| !c.is_ascii()) {
            panic!("can't escape non-ascii string {val}");
        }

        val.chars().fold(String::new(), |mut output, c| {
            let _ = write!(output, "\\x{:02x}", c as u8);
            output
        })
    }
}

#[derive(Serialize)]
struct ExtractedData {
    pub static_qstrs: Vec<QStr>,
    pub unsorted_qstrs: Vec<QStr>,
}

pub struct Artifacts {
    include_dir: PathBuf,
    lib_dir: PathBuf,
    libs: Vec<String>,
}

fn add_src<P: AsRef<Path>>(builder: &mut cc::Build, path: P) {
    let path = path.as_ref();
    let slash_path = path.to_slash_lossy();
    println!("cargo::rerun-if-changed={slash_path}");
    builder.file(&*slash_path);
}

fn add_src_dir<P: AsRef<Path>>(
    source_files: &mut Vec<PathBuf>,
    header_files: &mut Vec<PathBuf>,
    dir: P,
) {
    let dir = dir.as_ref();
    for entry in dir.read_dir().expect("directory exists") {
        let entry_path = entry.expect("entry is valid").path();
        // let slash_path = entry_path.to_slash_lossy();
        let Some(extension) = entry_path.extension() else {
            continue;
        };

        if extension == "h" {
            header_files.push(entry_path.clone());
            //           println!("cargo::rerun-if-changed={slash_path}");
        }
        if extension == "c" {
            source_files.push(entry_path.clone());
            //println!("cargo::rerun-if-changed={slash_path}");
            //           builder.file(&*slash_path);
        }
    }
}

impl Build {
    #[allow(clippy::new_without_default)]
    pub fn new(config: Config) -> Self {
        let out_dir = env::var_os("OUT_DIR")
            .map(|s| PathBuf::from(s).join("micropython-build"))
            .expect("$OUT_DIR is set");
        // let target = env::var("TARGET").expect("$TARGET set");
        // let host = env::var("HOST").expect("$HOST is set");

        let lib_dir = out_dir.join("lib");
        let include_dir = out_dir.join("include");
        let source_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("third_party/micropython");

        let mut source_files = Vec::new();
        let mut header_files = Vec::new();

        add_src_dir(&mut source_files, &mut header_files, source_dir.join("py"));

        source_files.push(source_dir.join("shared/runtime/gchelper_generic.c"));

        Build {
            out_dir,
            lib_dir,
            include_dir,
            source_dir,
            source_files,
            header_files,
            config,
            data: Data::new(),
        }
    }

    fn gen_mpconfigport<W: Write>(&self, mut w: W) {
        static TEMPLATE: &str = include_str!("../templates/mpconfigport.h.tmpl");
        let mut reg = Handlebars::new();

        w.write_all(
            reg.render_template(TEMPLATE, &())
                .expect("template renders")
                .as_bytes(),
        )
        .expect("can write to template");
    }

    fn lib_dir(&self) -> PathBuf {
        self.out_dir.join("lib")
    }

    fn include_dir(&self) -> PathBuf {
        self.out_dir.join("include")
    }

    fn builder(&self) -> cc::Build {
        let mut builder = cc::Build::new();
        builder.warnings(false);
        // .opt_level(2)
        // .cargo_metadata(false);
        builder.flag("-w"); // Disable warnings
        builder.include(".");
        builder.include(&self.include_dir);
        builder.include(&self.source_dir);

        builder
    }

    fn extract_data(&self) -> ExtractedData {
        let qstr_re = Regex::new(r"MP_QSTR_([_a-zA-Z0-9]+)").expect("regex compiles");
        let mut idents: HashSet<String> = self
            .data
            .static_qstrs
            .iter()
            .chain(self.data.unsorted_qstrs.iter())
            .map(|s| {
                QStr::new(
                    &self.config,
                    &self.data,
                    s,
                    0,
                    "Built in statics".to_string(),
                )
                .ident
            })
            .collect();
        let mut qstrs: Vec<_> = self
            .data
            .unsorted_qstrs
            .iter()
            .map(|s| {
                QStr::new(
                    &self.config,
                    &self.data,
                    s,
                    0,
                    "Built in unsorted".to_string(),
                )
            })
            .collect();
        for source in &self.source_files {
            let mut builder = self.builder();
            builder.file(source);
            let preprocessed_bytes = builder.expand();
            let preprocessed = String::from_utf8_lossy(&preprocessed_bytes);
            for line in preprocessed.lines() {
                for (_, [s]) in qstr_re.captures_iter(line).map(|c| c.extract()) {
                    let qstr = QStr::new(
                        &self.config,
                        &self.data,
                        s,
                        1,
                        source.to_string_lossy().to_string(),
                    );
                    if !idents.contains(&qstr.ident) {
                        idents.insert(qstr.ident.clone());
                        qstrs.push(qstr);
                    }
                }
            }
        }

        let static_qstrs: Vec<_> = self
            .data
            .static_qstrs
            .iter()
            .map(|s| {
                QStr::new(
                    &self.config,
                    &self.data,
                    s,
                    0,
                    "Built in unsorted".to_string(),
                )
            })
            .collect();

        ExtractedData {
            static_qstrs,
            unsorted_qstrs: qstrs,
        }
    }

    pub fn build(&mut self) -> Artifacts {
        if self.lib_dir.exists() {
            fs::remove_dir_all(&self.lib_dir).unwrap();
        }
        fs::create_dir_all(&self.lib_dir).unwrap();

        if self.include_dir.exists() {
            fs::remove_dir_all(&self.include_dir).unwrap();
        }
        fs::create_dir_all(&self.include_dir).unwrap();
        fs::create_dir_all(self.include_dir.join("genhdr")).unwrap();

        let mut mpconfigport = File::create(self.include_dir.join("mpconfigport.h"))
            .expect("mpconfigport.h should be created");
        self.gen_mpconfigport(&mut mpconfigport);

        const GEN_HEADERS: &[(&str, &str)] = &[
            (
                "genhdr/root_pointers.h",
                include_str!("../templates/root_pointers.h.tmpl"),
            ),
            (
                "genhdr/qstrdefs.generated.h",
                include_str!("../templates/qstrdefs.generated.h.tmpl"),
            ),
            (
                "genhdr/moduledefs.h",
                include_str!("../templates/moduledefs.h.tmpl"),
            ),
            (
                "genhdr/mpversion.h",
                include_str!("../templates/mpversion.h.tmpl"),
            ),
            ("mphalport.h", include_str!("../templates/mphalport.h.tmpl")),
            (
                "mpconfigport.h",
                include_str!("../templates/mpconfigport.h.tmpl"),
            ),
        ];

        // Create empty files so the preprocessor can find them while we extract
        // data from the sources.
        for (header, _) in GEN_HEADERS {
            let _file = File::create(self.include_dir.join(header))
                .unwrap_or_else(|_| panic!("can create {header}"));
        }
        let data = self.extract_data();

        let reg = Handlebars::new();
        for (header, template) in GEN_HEADERS {
            let mut file = File::create(self.include_dir.join(header))
                .unwrap_or_else(|_| panic!("can create {header}"));
            file.write_all(
                reg.render_template(template, &data)
                    .unwrap_or_else(|e| panic!("Can't render template for {header}: {e}"))
                    .as_bytes(),
            )
            .unwrap_or_else(|_| panic!("Can't write {header}"));
        }

        let mut builder = self.builder();
        for source in &self.source_files {
            builder.file(source);
        }

        let lib_name = "micropython";
        builder.out_dir(&self.lib_dir).compile(lib_name);

        Artifacts {
            lib_dir: self.lib_dir.clone(),
            include_dir: self.include_dir.clone(),
            libs: vec![lib_name.to_string()],
        }
    }
}

#[cfg(test)]
mod tests {
    use regex::Captures;

    use super::*;
    fn qstr_test_data() -> Vec<QStr> {
        let data = include_str!("test_data/qstrdefs.generated.h");
        let qdef_re =
            Regex::new(r#"QDEF([01])\((MP_QSTR_[_a-zA-Z0-9]+), ([0-9]+), ([0-9]+), "(.*)"\)"#)
                .expect("regex compiles");
        let escape_re = Regex::new(r"\\x([0-9a-f]){2}").expect("regex compiles");
        let mut qstrs = Vec::new();
        for line in data.lines() {
            for (_, [pool, ident, hash, val_len, val]) in
                qdef_re.captures_iter(line).map(|c| c.extract())
            {
                let pool = pool.parse().unwrap();
                let hash = hash.parse().unwrap();
                let val_len = val_len.parse().unwrap();
                let val = escape_re.replace(val, |caps: &Captures| {
                    format!("{}", u8::from_str_radix(&caps[1], 16).unwrap() as char)
                });
                qstrs.push(QStr {
                    pool,
                    ident: ident.to_string(),
                    hash,
                    val_len,
                    val: val.to_string(),
                    source: "".to_string(),
                });
            }
        }

        qstrs
    }

    #[test]
    fn qstrs_compute_metadata_correctly() {
        let test_data = qstr_test_data();
        let data = Data::new();
        let config: Config = Default::default();
        for test in test_data {
            let qstr = QStr::new(&config, &data, &test.val, test.pool, "".to_string());
            assert_eq!(
                qstr.hash, test.hash,
                "Incorrect hash {:x} for {:x?}",
                qstr.hash, test
            );
            assert_eq!(
                qstr.ident, test.ident,
                "Incorrect ident {} for {:x?}",
                qstr.ident, test
            );
            assert_eq!(
                qstr.val_len, test.val_len,
                "Incorrect length {} for {:x?}",
                qstr.val_len, test
            );
        }
    }
}
