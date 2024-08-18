use std::num::Wrapping;
use std::{collections::HashSet, fmt::Write as _};

use anyhow::Result;
use regex::Regex;
use serde::Serialize;

use super::{BytesIn, Config, Data};

#[derive(Debug, Serialize)]
pub struct QStr {
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

pub struct ExtractedQstrs {
    pub static_qstrs: Vec<QStr>,
    pub unsorted_qstrs: Vec<QStr>,
}

pub struct Extractor<'a> {
    config: &'a Config,
    data: &'a Data,
    re: Regex,
    idents: HashSet<String>,
    unsorted_qstrs: Vec<QStr>,
}

impl<'a> Extractor<'a> {
    pub fn new(config: &'a Config, data: &'a Data) -> Result<Self> {
        let re = Regex::new(r"MP_QSTR_([_a-zA-Z0-9]+)")?;

        let idents: HashSet<String> = data
            .static_qstrs
            .iter()
            .chain(data.unsorted_qstrs.iter())
            .map(|s| QStr::new(config, data, s, 0, "Built in statics".to_string()).ident)
            .collect();
        let unsorted_qstrs: Vec<_> = data
            .unsorted_qstrs
            .iter()
            .map(|s| QStr::new(config, data, s, 0, "Built in unsorted".to_string()))
            .collect();

        Ok(Self {
            config,
            data,
            re,
            idents,
            unsorted_qstrs,
        })
    }

    pub fn process_line(&mut self, source: &str, line: &str) -> Result<()> {
        for (_, [s]) in self.re.captures_iter(line).map(|c| c.extract()) {
            let qstr = QStr::new(self.config, self.data, s, 1, source.to_string());
            if !self.idents.contains(&qstr.ident) {
                self.idents.insert(qstr.ident.clone());
                self.unsorted_qstrs.push(qstr);
            }
        }
        Ok(())
    }

    pub fn finish(self) -> ExtractedQstrs {
        let static_qstrs: Vec<_> = self
            .data
            .static_qstrs
            .iter()
            .map(|s| {
                QStr::new(
                    self.config,
                    self.data,
                    s,
                    0,
                    "Built in unsorted".to_string(),
                )
            })
            .collect();

        ExtractedQstrs {
            static_qstrs,
            unsorted_qstrs: self.unsorted_qstrs,
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
