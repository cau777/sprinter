use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub const WEB_SEARCH: &str = "openrouter:web_search";
pub const BASH: &str = "openrouter:bash";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
pub struct Citation {
    pub url: String,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
pub struct ToolStep {
    pub id: String,
    pub tool: String,
    pub offset: usize,
    pub status: String,
    #[ts(type = "unknown | null")]
    pub input: Option<Value>,
    pub output: Option<ToolStepOutput>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
pub struct ToolStepOutput {
    pub stdout: String,
    pub stderr: String,
    #[ts(type = "number | null")]
    pub exit_code: Option<i64>,
}

pub fn validate_enabled_tools(tools: &[String]) -> Result<(), ToolValidationError> {
    let mut seen = std::collections::HashSet::new();
    for tool in tools {
        if !matches!(tool.as_str(), WEB_SEARCH | BASH) || !seen.insert(tool) {
            return Err(ToolValidationError::Unknown);
        }
    }
    if tools.iter().any(|tool| tool == WEB_SEARCH) && tools.iter().any(|tool| tool == BASH) {
        return Err(ToolValidationError::Incompatible);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolValidationError {
    Unknown,
    Incompatible,
}

#[cfg(test)]
mod tests {
    use super::{BASH, WEB_SEARCH, validate_enabled_tools};

    #[test]
    fn web_search_and_bash_cannot_be_combined() {
        assert_eq!(
            validate_enabled_tools(&[WEB_SEARCH.into(), BASH.into()]),
            Err(super::ToolValidationError::Incompatible)
        );
    }

    #[test]
    fn rejects_unknown_and_duplicate_tools() {
        assert!(validate_enabled_tools(&["unknown".into()]).is_err());
        assert!(validate_enabled_tools(&[WEB_SEARCH.into(), WEB_SEARCH.into()]).is_err());
    }
}

pub fn decode_json_column<T: for<'de> Deserialize<'de>>(value: Option<&str>) -> T
where
    T: Default,
{
    value
        .and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or_default()
}

macro_rules! json_column_module {
    ($module:ident, $value_type:ty) => {
        pub mod $module {
            use serde::{Deserialize, Deserializer, Serialize, Serializer};

            pub fn serialize<S: Serializer>(
                value: &Option<String>,
                serializer: S,
            ) -> Result<S::Ok, S::Error> {
                match value {
                    Some(value) => {
                        let parsed = serde_json::from_str::<$value_type>(value)
                            .map_err(<S::Error as serde::ser::Error>::custom)?;
                        parsed.serialize(serializer)
                    }
                    None => serializer.serialize_none(),
                }
            }

            pub fn deserialize<'de, D: Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Option<String>, D::Error> {
                let value = Option::<$value_type>::deserialize(deserializer)?;
                value
                    .map(|value| {
                        serde_json::to_string(&value)
                            .map_err(<D::Error as serde::de::Error>::custom)
                    })
                    .transpose()
            }
        }
    };
}

json_column_module!(string_vec_column, Vec<String>);
json_column_module!(citation_vec_column, Vec<super::Citation>);
json_column_module!(tool_step_vec_column, Vec<super::ToolStep>);
