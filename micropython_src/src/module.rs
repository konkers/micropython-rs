use anyhow::Result;
use regex::Regex;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Module {
    pub qstr_ident: String,
    pub upper_name: String,
    pub symbol: String,
    pub source: String,
}

pub struct ExtractedModules {
    pub modules: Vec<Module>,
    pub extensible_modules: Vec<Module>,
    pub module_delegations: Vec<Module>,
}

pub struct Extractor {
    re: Regex,
    modules: Vec<Module>,
    extensible_modules: Vec<Module>,
    module_delegations: Vec<Module>,
}

impl Extractor {
    pub fn new() -> Result<Self> {
        let re = Regex::new(
            r"(MP_REGISTER_MODULE|MP_REGISTER_EXTENSIBLE_MODULE|MP_REGISTER_MODULE_DELEGATION)\((.*?),\s*(.*?)\);",
        )?;

        Ok(Self {
            re,
            modules: Vec::new(),
            extensible_modules: Vec::new(),
            module_delegations: Vec::new(),
        })
    }

    pub fn process_line(&mut self, source: &str, line: &str) -> Result<()> {
        for (_, [ty, qstr, symbol]) in self.re.captures_iter(line).map(|c| c.extract()) {
            let upper_name = qstr.strip_prefix("MP_QSTR_").unwrap_or(qstr);
            let module = Module {
                qstr_ident: qstr.to_string(),
                upper_name: upper_name.to_uppercase(),
                symbol: symbol.to_string(),
                source: source.to_string(),
            };
            match ty {
                "MP_REGISTER_MODULE" => self.modules.push(module),
                "MP_REGISTER_EXTENSIBLE_MODULE" => self.extensible_modules.push(module),
                "MP_REGISTER_MODULE_DELEGATION" => self.module_delegations.push(module),
                _ => panic!("Unextpected module type {ty}"),
            }
        }

        Ok(())
    }

    pub fn finish(self) -> ExtractedModules {
        ExtractedModules {
            modules: self.modules,
            extensible_modules: self.extensible_modules,
            module_delegations: self.module_delegations,
        }
    }
}
