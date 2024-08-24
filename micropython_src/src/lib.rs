use std::{
    collections::HashMap,
    env,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Result;
use handlebars::Handlebars;
use serde::Serialize;

mod module;
mod qstr;

use module::Module;
use qstr::QStr;

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

#[derive(Debug, Serialize)]
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

#[derive(Debug, Default, Serialize)]
pub struct Config {
    pub bytes_in_hash: BytesIn,
    pub bytes_in_string: BytesIn,
    pub extra_qstrs: Vec<String>,
}

impl Config {
    pub fn qstr(mut self, qstr: &str) -> Self {
        self.extra_qstrs.push(qstr.to_string());
        self
    }
    fn is_header_used(&self, path: &Path) -> bool {
        for suffix in [
            "py/dynruntime.h",
            "py/grammar.h",
            "py/qstrdefs.h",
            "py/vmentrytable.h",
        ] {
            if path.ends_with(suffix) {
                return false;
            }
        }

        true
    }
}

pub struct Build {
    lib_dir: PathBuf,
    include_dir: PathBuf,
    source_dir: PathBuf,
    source_files: Vec<PathBuf>,
    header_files: Vec<PathBuf>,
    data: Data,
    config: Config,
}

#[derive(Serialize)]
struct ExtractedData {
    pub static_qstrs: Vec<QStr>,
    pub unsorted_qstrs: Vec<QStr>,
    pub all_qstrs: Vec<QStr>,
    pub modules: Vec<Module>,
    pub extensible_modules: Vec<Module>,
    pub module_delegations: Vec<Module>,
}

impl Build {
    #[allow(clippy::new_without_default)]
    pub fn new(config: Config) -> Self {
        let out_dir = env::var_os("OUT_DIR")
            .map(|s| PathBuf::from(s).join("micropython-build"))
            .expect("$OUT_DIR is set");

        let lib_dir = out_dir.join("lib");
        let include_dir = out_dir.join("include");
        let source_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("third_party/micropython");

        let mut source_files = Vec::new();
        let mut header_files = Vec::new();

        Self::add_src_dir(
            &config,
            &mut source_files,
            &mut header_files,
            source_dir.join("py"),
        );
        source_files.push(source_dir.join("shared/runtime/gchelper_generic.c"));
        header_files.push(source_dir.join("shared/runtime/gchelper.h"));

        Build {
            lib_dir,
            include_dir,
            source_dir,
            source_files,
            header_files,
            config,
            data: Data::new(),
        }
    }

    fn add_src_dir<P: AsRef<Path>>(
        config: &Config,
        source_files: &mut Vec<PathBuf>,
        header_files: &mut Vec<PathBuf>,
        dir: P,
    ) {
        let dir = dir.as_ref();
        for entry in dir.read_dir().expect("directory exists") {
            let entry_path = entry.expect("entry is valid").path();
            let Some(extension) = entry_path.extension() else {
                continue;
            };

            if extension == "h" && config.is_header_used(&entry_path) {
                header_files.push(entry_path.clone());
            }
            if extension == "c" {
                source_files.push(entry_path.clone());
                //println!("cargo::rerun-if-changed={slash_path}");
                //           builder.file(&*slash_path);
            }
        }
    }
    fn include_dirs<Callback: FnMut(&PathBuf)>(&self, mut callback: Callback) {
        callback(&PathBuf::from("."));
        callback(&self.include_dir);
        callback(&self.source_dir);
    }

    fn builder(&self) -> cc::Build {
        let mut builder = cc::Build::new();
        builder.warnings(false);
        builder.flag("-w"); // Disable warnings
        self.include_dirs(|path| {
            builder.include(path);
        });

        builder
    }

    fn extract_data(&self) -> Result<ExtractedData> {
        let mut qstr_extractor = qstr::Extractor::new(&self.config, &self.data)?;
        let mut module_extractor = module::Extractor::new()?;
        for source in &self.source_files {
            let mut builder = self.builder();
            builder.define("NO_QSTR", ""); // Needed to keep MP_REGISTER_MODULE* macros from being defined.
            builder.file(source);
            let preprocessed_bytes = builder.expand();
            let preprocessed = String::from_utf8_lossy(&preprocessed_bytes);

            for line in preprocessed.lines() {
                let source = source.strip_prefix(&self.source_dir)?.to_string_lossy();
                qstr_extractor.process_line(&source, line)?;
                module_extractor.process_line(&source, line)?;
            }
        }

        let mut qstrs = qstr_extractor.finish();
        let modules = module_extractor.finish();

        for qstr in &self.config.extra_qstrs {
            qstrs.unsorted_qstrs.push(QStr::new(
                &self.config,
                &self.data,
                qstr,
                1,
                "Rust".to_string(),
            ));
        }

        let all_qstrs = qstrs
            .static_qstrs
            .iter()
            .chain(qstrs.unsorted_qstrs.iter())
            .cloned()
            .collect();

        Ok(ExtractedData {
            static_qstrs: qstrs.static_qstrs,
            unsorted_qstrs: qstrs.unsorted_qstrs,
            all_qstrs,
            modules: modules.modules,
            extensible_modules: modules.extensible_modules,
            module_delegations: modules.module_delegations,
        })
    }

    fn generate_headers(&mut self) -> Result<ExtractedData> {
        if self.lib_dir.exists() {
            fs::remove_dir_all(&self.lib_dir)?;
        }
        fs::create_dir_all(&self.lib_dir)?;

        if self.include_dir.exists() {
            fs::remove_dir_all(&self.include_dir)?;
        }
        fs::create_dir_all(&self.include_dir)?;
        fs::create_dir_all(self.include_dir.join("genhdr"))?;

        // TODO - Copy headers into `include_dir` and remove direct source include paths from
        // compile.

        let reg = Handlebars::new();
        // Create mpconfigport.h first so that data extraction is configured
        // correctly.
        {
            let mut file = File::create(self.include_dir.join("mpconfigport.h"))?;
            file.write_all(
                reg.render_template(
                    include_str!("../templates/mpconfigport.h.tmpl"),
                    &self.config,
                )?
                .as_bytes(),
            )?
        }

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
        ];

        // Create empty files so the preprocessor can find them while we extract
        // data from the sources.
        for (header, _) in GEN_HEADERS {
            let _file = File::create(self.include_dir.join(header))?;
        }
        let data = self.extract_data()?;

        for (header, template) in GEN_HEADERS {
            let mut file = File::create(self.include_dir.join(header))?;
            file.write_all(reg.render_template(template, &data)?.as_bytes())?
        }

        Ok(data)
    }

    pub fn build(&mut self) -> Result<()> {
        let data = self.generate_headers()?;

        let mut builder = self.builder();
        for source in &self.source_files {
            builder.file(source);
        }

        let lib_name = "micropython";
        builder.out_dir(&self.lib_dir).compile(lib_name);

        // Generate project specific qstr rust bindings.
        let reg = Handlebars::new();
        include_str!("../templates/qstr.rs.tmpl");
        let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
        let mut file = File::create(out_path.join("qstr.rs"))?;
        file.write_all(
            reg.render_template(include_str!("../templates/qstr.rs.tmpl"), &data)?
                .as_bytes(),
        )?;

        Ok(())
    }

    pub fn bindgen(&mut self) -> Result<()> {
        self.generate_headers()?;

        let mut clang_args = Vec::new();
        self.include_dirs(|path| clang_args.push(format!("-I{}", path.to_string_lossy())));

        let header_files: Vec<String> = self
            .header_files
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect();

        let bindings = bindgen::Builder::default()
            .use_core()
            .clang_args(clang_args)
            .headers(header_files)
            .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
            // Finish the builder and generate the bindings.
            .generate()?;

        let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
        bindings.write_to_file(out_path.join("micropython-bindings.rs"))?;

        Ok(())
    }
}
