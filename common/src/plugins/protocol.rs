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
pub enum ConfigValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Datetime(String),
    Array(Vec<ConfigValue>),
    Table(std::collections::BTreeMap<String, ConfigValue>),
}
impl From<&crate::config::ConfigValue> for ConfigValue {
    fn from(value: &crate::config::ConfigValue) -> Self {
        use crate::config::ConfigValue as V;
        match value {
            V::String(v) => Self::String(v.clone()),
            V::Integer(v) => Self::Integer(*v),
            V::Float(v) => Self::Float(*v),
            V::Boolean(v) => Self::Boolean(*v),
            V::Datetime(v) => Self::Datetime(v.to_string()),
            V::Array(v) => Self::Array(v.iter().map(Self::from).collect()),
            V::Table(v) => Self::Table(v.iter().map(|(k, v)| (k.clone(), Self::from(v))).collect()),
        }
    }
}
impl ConfigValue {
    fn into_toml(self) -> Result<crate::config::ConfigValue, String> {
        use crate::config::ConfigValue as V;
        Ok(match self {
            Self::String(v) => V::String(v),
            Self::Integer(v) => V::Integer(v),
            Self::Float(v) => V::Float(v),
            Self::Boolean(v) => V::Boolean(v),
            Self::Datetime(v) => V::Datetime(
                v.parse()
                    .map_err(|e| format!("Invalid config datetime: {e}"))?,
            ),
            Self::Array(v) => V::Array(
                v.into_iter()
                    .map(Self::into_toml)
                    .collect::<Result<_, _>>()?,
            ),
            Self::Table(v) => V::Table(
                v.into_iter()
                    .map(|(k, v)| Ok((k, v.into_toml()?)))
                    .collect::<Result<_, String>>()?,
            ),
        })
    }
}
#[derive(Serialize, Deserialize)]
pub struct Initialization {
    pub parser: crate::config::ParserConfig,
    pub formatter: crate::config::FormatterConfig,
    pub plugin_path: String,
    pub plugins: Vec<String>,
    pub plugin_config: std::collections::BTreeMap<String, ConfigValue>,
    pub debug: crate::config::DebugConfig,
    #[serde(with = "path")]
    pub workdir: PathBuf,
}
impl Initialization {
    pub fn new(config: &Config) -> Self {
        Self {
            parser: config.parser.clone(),
            formatter: config.formatter.clone(),
            plugin_path: config.plugins.path.clone(),
            plugins: config.plugins.plugins.clone(),
            plugin_config: config
                .plugins
                .config
                .iter()
                .map(|(k, v)| (k.clone(), v.into()))
                .collect(),
            debug: config.debug.clone(),
            workdir: config.workdir.clone(),
        }
    }
    pub fn into_config(self) -> Result<Config, String> {
        let mut config = Config::default_with_workdir(self.workdir);
        config.parser = self.parser;
        config.formatter = self.formatter;
        config.debug = self.debug;
        config.plugins.path = self.plugin_path;
        config.plugins.plugins = self.plugins;
        config.plugins.config = self
            .plugin_config
            .into_iter()
            .map(|(k, v)| Ok((k, v.into_toml()?)))
            .collect::<Result<_, String>>()?;
        Ok(config)
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
    pub location: crate::TextRange,
}

impl From<&XenoDiagnostic> for Diagnostic {
    fn from(diagnostic: &XenoDiagnostic) -> Self {
        Self {
            message: diagnostic.message.clone(),
            severity: diagnostic.severity as u32,
            location: diagnostic.location,
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
            value => Err(format!("Invalid plugin diagnostic severity: {value}")),
        }
    }
}

pub mod json_schema {
    use serde::{Deserialize, Serialize};
    pub fn serialize<S: serde::Serializer>(
        schema: &Option<serde_json::Value>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        schema
            .as_ref()
            .map(serde_json::Value::to_string)
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<serde_json::Value>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|s| serde_json::from_str(&s).map_err(serde::de::Error::custom))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binary_config_and_module_requests_roundtrip() {
        let mut config = Config::default();
        config.plugins.config.insert(
            "fixture".into(),
            toml::from_str::<crate::config::ConfigValue>(
                "nested = [{ value = 42 }, { value = 0.5 }]\nactive = true\nwhen = 2026-10-13",
            )
            .unwrap(),
        );
        let bytes = xenomorph_plugin_api::codec::encode(&Initialization::new(&config)).unwrap();
        let decoded = xenomorph_plugin_api::codec::decode::<Initialization>(&bytes)
            .unwrap()
            .into_config()
            .unwrap();
        assert_eq!(config.plugins.config, decoded.plugins.config);
        assert_eq!(config.workdir, decoded.workdir);
        let source="type TreeNode = { children: Tree[] }; type Tree = |TreeNode|number; type Big = 999999999999999999999999999999; type Decimal = 1.123456789012345678901234567890;";
        let tokens = crate::lexer::Lexer::tokenize(source).unwrap();
        let (ast, errors) = crate::parser::Parser::parse(&tokens, source);
        assert!(errors.is_empty());
        let mut hierarchy = crate::semantic::TypeHierarchy::default();
        hierarchy.insert_metadata(&crate::semantic::MetadataGraph::from_all_descriptors(
            crate::semantic::BUILTIN_TYPES,
            crate::semantic::BUILTIN_TRAITS,
            crate::semantic::BUILTIN_ANNOTATIONS,
        ));
        let metadata = crate::semantic::MetadataGraph::from_all_descriptors(
            crate::semantic::BUILTIN_TYPES,
            crate::semantic::BUILTIN_TRAITS,
            crate::semantic::BUILTIN_ANNOTATIONS,
        );
        let scope = ScopeInfo {
            module_path: "test".into(),
            abs_path: PathBuf::new(),
            own_types: vec![],
            imported_types: std::collections::HashMap::new(),
            builtin_types: metadata.types.keys().cloned().collect(),
            known_annotations: metadata.annotations.keys().cloned().collect(),
            type_hierarchy: hierarchy,
            annotations: metadata.annotations,
        };
        let encoded = super::super::encode_request(false, source, &ast, &scope).unwrap();
        let request = xenomorph_plugin_api::codec::decode::<ModuleRequest>(&encoded).unwrap();
        request.scope.validate().unwrap();
        super::super::syntax::validate(&request.syntax.source, &request.syntax.declarations)
            .unwrap();
        assert_eq!(
            serde_json::to_value(ast).unwrap(),
            serde_json::to_value(request.syntax.declarations).unwrap()
        );
        let mut trailing = encoded;
        trailing.push(0);
        assert!(xenomorph_plugin_api::codec::decode::<ModuleRequest>(&trailing).is_err());
    }
}
