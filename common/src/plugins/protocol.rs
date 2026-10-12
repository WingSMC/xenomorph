use crate::{config::Config, semantic::ScopeInfo, XenoDiagSeverity, XenoDiagnostic};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use xenomorph_plugin_api::syntax::Syntax;

pub mod path {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::path::{Path, PathBuf};

    #[derive(Serialize, Deserialize)]
    enum NativePath {
        Windows(Vec<u16>),
        Unix(Vec<u8>),
    }

    pub fn serialize<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            NativePath::Windows(path.as_os_str().encode_wide().collect()).serialize(serializer)
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            NativePath::Unix(path.as_os_str().as_bytes().to_vec()).serialize(serializer)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
        let path = NativePath::deserialize(deserializer)?;
        match path {
            #[cfg(windows)]
            NativePath::Windows(units) => {
                use std::os::windows::ffi::OsStringExt;
                Ok(std::ffi::OsString::from_wide(&units).into())
            }
            #[cfg(unix)]
            NativePath::Unix(bytes) => {
                use std::os::unix::ffi::OsStringExt;
                Ok(std::ffi::OsString::from_vec(bytes).into())
            }
            _ => Err(serde::de::Error::custom(
                "Plugin path belongs to a different target platform",
            )),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct Initialization {
    pub config: Config,
    #[serde(with = "path")]
    pub workdir: PathBuf,
}

impl Initialization {
    pub fn new(config: &Config) -> Self {
        Self {
            config: config.clone(),
            workdir: config.workdir.clone(),
        }
    }

    pub fn into_config(mut self) -> Config {
        self.config.workdir = self.workdir;
        self.config
    }
}

#[derive(Serialize, Deserialize)]
pub struct ModuleRequest {
    pub generation: bool,
    pub syntax: Syntax,
    pub scope: ScopeInfo,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub message: String,
    pub severity: u32,
    pub value: String,
    pub line: u32,
    pub column: u32,
}

impl From<&XenoDiagnostic<'_>> for Diagnostic {
    fn from(diagnostic: &XenoDiagnostic<'_>) -> Self {
        Self {
            message: diagnostic.message.clone(),
            severity: match diagnostic.severity {
                XenoDiagSeverity::Err => 0,
                XenoDiagSeverity::Warn => 1,
                XenoDiagSeverity::Info => 2,
            },
            value: diagnostic.location.v.to_string(),
            line: diagnostic.location.l,
            column: diagnostic.location.c,
        }
    }
}

impl Diagnostic {
    #[inline]
    pub fn get_as_xeno_severity(&self) -> Result<XenoDiagSeverity, String> {
        match self.severity {
            0 => Ok(XenoDiagSeverity::Err),
            1 => Ok(XenoDiagSeverity::Warn),
            2 => Ok(XenoDiagSeverity::Info),
            value => return Err(format!("Invalid plugin diagnostic severity: {value}")),
        }
    }
}
