use std::collections::HashMap;
use std::path::Path;

use plist::Value;
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum XctestRunError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist parse error: {0}")]
    Plist(#[from] plist::Error),
    #[error("missing __xctestrun_metadata__.FormatVersion")]
    MissingFormatVersion,
    #[error("unsupported .xctestrun format version {0}")]
    UnsupportedFormatVersion(i64),
    #[error("the provided .xctestrun file does not contain any test configurations")]
    EmptyConfigurations,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SchemeData {
    #[serde(rename = "TestHostBundleIdentifier", default)]
    pub test_host_bundle_identifier: String,
    #[serde(rename = "TestBundlePath", default)]
    pub test_bundle_path: String,
    #[serde(rename = "SkipTestIdentifiers", default)]
    pub skip_test_identifiers: Vec<String>,
    #[serde(rename = "OnlyTestIdentifiers", default)]
    pub only_test_identifiers: Vec<String>,
    #[serde(rename = "IsUITestBundle", default)]
    pub is_ui_test_bundle: bool,
    #[serde(rename = "CommandLineArguments", default)]
    pub command_line_arguments: Vec<String>,
    #[serde(rename = "EnvironmentVariables", default)]
    pub environment_variables: HashMap<String, Value>,
    #[serde(rename = "TestingEnvironmentVariables", default)]
    pub testing_environment_variables: HashMap<String, Value>,
    #[serde(rename = "UITargetAppEnvironmentVariables", default)]
    pub ui_target_app_environment_variables: HashMap<String, Value>,
    #[serde(rename = "UITargetAppCommandLineArguments", default)]
    pub ui_target_app_command_line_arguments: Vec<String>,
    #[serde(rename = "UITargetAppPath", default)]
    pub ui_target_app_path: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TestConfiguration {
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "TestTargets", default)]
    pub test_targets: Vec<SchemeData>,
}

pub fn parse_xctestrun_file(
    path: impl AsRef<Path>,
) -> Result<Vec<TestConfiguration>, XctestRunError> {
    let bytes = std::fs::read(path)?;
    parse_xctestrun_bytes(&bytes)
}

pub fn parse_xctestrun_bytes(bytes: &[u8]) -> Result<Vec<TestConfiguration>, XctestRunError> {
    let root = Value::from_reader_xml(bytes)
        .or_else(|_| Value::from_reader(std::io::Cursor::new(bytes)))?;
    let version = format_version(&root)?;
    match version {
        1 => parse_version_1(root),
        2 => parse_version_2(root),
        other => Err(XctestRunError::UnsupportedFormatVersion(other)),
    }
}

fn format_version(root: &Value) -> Result<i64, XctestRunError> {
    let dict = root
        .as_dictionary()
        .ok_or(XctestRunError::MissingFormatVersion)?;
    dict.get("__xctestrun_metadata__")
        .and_then(Value::as_dictionary)
        .and_then(|metadata| metadata.get("FormatVersion"))
        .and_then(Value::as_signed_integer)
        .ok_or(XctestRunError::MissingFormatVersion)
}

fn parse_version_1(root: Value) -> Result<Vec<TestConfiguration>, XctestRunError> {
    let dict = root
        .into_dictionary()
        .ok_or(XctestRunError::MissingFormatVersion)?;
    for (key, value) in dict {
        if key == "__xctestrun_metadata__" {
            continue;
        }

        let scheme: SchemeData = plist::from_value(&value)?;
        return Ok(vec![TestConfiguration {
            name: String::new(),
            test_targets: vec![scheme],
        }]);
    }

    Err(XctestRunError::EmptyConfigurations)
}

fn parse_version_2(root: Value) -> Result<Vec<TestConfiguration>, XctestRunError> {
    #[derive(Deserialize)]
    struct Version2Root {
        #[serde(rename = "TestConfigurations", default)]
        test_configurations: Vec<TestConfiguration>,
    }

    let parsed: Version2Root = plist::from_value(&root)?;
    if parsed.test_configurations.is_empty() {
        return Err(XctestRunError::EmptyConfigurations);
    }
    Ok(parsed.test_configurations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_1_xctestrun() {
        let plist = br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>DemoApp</key>
  <dict>
    <key>TestHostBundleIdentifier</key><string>com.example.DemoAppUITests.xctrunner</string>
    <key>TestBundlePath</key><string>DemoAppUITests.xctest</string>
    <key>IsUITestBundle</key><true/>
    <key>CommandLineArguments</key><array><string>-ApplePersistenceIgnoreState</string></array>
  </dict>
  <key>__xctestrun_metadata__</key>
  <dict><key>FormatVersion</key><integer>1</integer></dict>
</dict>
</plist>"#;

        let configs = parse_xctestrun_bytes(plist).unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(
            configs[0].test_targets[0].test_host_bundle_identifier,
            "com.example.DemoAppUITests.xctrunner"
        );
        assert!(configs[0].test_targets[0].is_ui_test_bundle);
    }

    #[test]
    fn parses_version_2_xctestrun() {
        let plist = br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>TestConfigurations</key>
  <array>
    <dict>
      <key>Name</key><string>UITests</string>
      <key>TestTargets</key>
      <array>
        <dict>
          <key>TestHostBundleIdentifier</key><string>com.example.DemoAppUITests.xctrunner</string>
          <key>TestBundlePath</key><string>DemoAppUITests.xctest</string>
          <key>IsUITestBundle</key><true/>
        </dict>
      </array>
    </dict>
  </array>
  <key>__xctestrun_metadata__</key>
  <dict><key>FormatVersion</key><integer>2</integer></dict>
</dict>
</plist>"#;

        let configs = parse_xctestrun_bytes(plist).unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "UITests");
        assert_eq!(
            configs[0].test_targets[0].test_bundle_path,
            "DemoAppUITests.xctest"
        );
    }

    #[test]
    fn parses_ui_target_app_command_line_arguments() {
        let plist = br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>DemoApp</key>
  <dict>
    <key>TestHostBundleIdentifier</key><string>com.example.DemoAppUITests.xctrunner</string>
    <key>TestBundlePath</key><string>DemoAppUITests.xctest</string>
    <key>IsUITestBundle</key><true/>
    <key>UITargetAppCommandLineArguments</key>
    <array>
      <string>-AppleLanguages</string>
      <string>(en)</string>
    </array>
  </dict>
  <key>__xctestrun_metadata__</key>
  <dict><key>FormatVersion</key><integer>1</integer></dict>
</dict>
</plist>"#;

        let configs = parse_xctestrun_bytes(plist).unwrap();
        assert_eq!(
            configs[0].test_targets[0].ui_target_app_command_line_arguments,
            vec!["-AppleLanguages".to_string(), "(en)".to_string()]
        );
    }
}
