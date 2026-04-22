use std::collections::HashMap;

use ios_proto::nskeyedarchiver_encode::{NsUrl, XcTestConfiguration, XctCapabilities};
use plist::Value;
use uuid::Uuid;

use super::xctestrun::SchemeData;

const TARGET_APP_ENV_KEY: &str = "__IOS_TUNNEL_TARGET_APP_ENV_JSON";
const TARGET_APP_ARGS_KEY: &str = "__IOS_TUNNEL_TARGET_APP_ARGS_JSON";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledAppInfo {
    pub bundle_id: String,
    pub path: String,
    pub executable: String,
    pub container: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestLaunchPlan {
    pub runner: InstalledAppInfo,
    pub target: Option<InstalledAppInfo>,
    pub xctest_bundle_name: String,
    pub is_xctest: bool,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub tests_to_run: Vec<String>,
    pub tests_to_skip: Vec<String>,
}

impl TestLaunchPlan {
    pub fn from_scheme(
        scheme: &SchemeData,
        runner: InstalledAppInfo,
        target: Option<InstalledAppInfo>,
    ) -> Self {
        let mut env = HashMap::new();
        merge_string_values(&mut env, &scheme.environment_variables);
        merge_string_values(&mut env, &scheme.testing_environment_variables);
        merge_string_values(&mut env, &scheme.ui_target_app_environment_variables);
        store_target_app_context(
            &mut env,
            &scheme.ui_target_app_environment_variables,
            &scheme.ui_target_app_command_line_arguments,
        );

        Self {
            runner,
            target,
            xctest_bundle_name: bundle_name_from_path(&scheme.test_bundle_path),
            is_xctest: !scheme.is_ui_test_bundle,
            args: scheme.command_line_arguments.clone(),
            env,
            tests_to_run: scheme.only_test_identifiers.clone(),
            tests_to_skip: scheme.skip_test_identifiers.clone(),
        }
    }

    pub fn test_bundle_path(&self) -> String {
        format!("{}/PlugIns/{}", self.runner.path, self.xctest_bundle_name)
    }

    pub fn xctest_configuration(
        &self,
        product_major_version: u64,
        session_identifier: Uuid,
    ) -> XcTestConfiguration {
        let automation_framework_path = if product_major_version >= 17 {
            "/System/Developer/Library/PrivateFrameworks/XCTAutomationSupport.framework"
        } else {
            "/Developer/Library/PrivateFrameworks/XCTAutomationSupport.framework"
        };

        let mut additional_fields = vec![
            ("reportActivities".to_string(), Value::Boolean(true)),
            (
                "testApplicationDependencies".to_string(),
                Value::Dictionary(Default::default()),
            ),
        ];

        if let Some(target) = &self.target {
            additional_fields.push((
                "productModuleName".to_string(),
                Value::String(product_module_name(&self.xctest_bundle_name)),
            ));
            additional_fields.push((
                "targetApplicationBundleID".to_string(),
                Value::String(target.bundle_id.clone()),
            ));
            additional_fields.push((
                "targetApplicationPath".to_string(),
                Value::String(target.path.clone()),
            ));
            additional_fields.push((
                "targetApplicationArguments".to_string(),
                Value::Array(
                    self.target_application_arguments()
                        .into_iter()
                        .map(Value::String)
                        .collect(),
                ),
            ));
            additional_fields.push((
                "targetApplicationEnvironment".to_string(),
                Value::Dictionary(self.target_application_environment()),
            ));
        }

        if !self.tests_to_run.is_empty() {
            additional_fields.push((
                "testsToRun".to_string(),
                Value::Array(
                    self.tests_to_run
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            ));
        }
        if !self.tests_to_skip.is_empty() {
            additional_fields.push((
                "testsToSkip".to_string(),
                Value::Array(
                    self.tests_to_skip
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            ));
        }

        XcTestConfiguration {
            session_identifier,
            test_bundle_url: NsUrl {
                path: self.test_bundle_path(),
            },
            ide_capabilities: default_capabilities(),
            automation_framework_path: automation_framework_path.to_string(),
            initialize_for_ui_testing: !self.is_xctest,
            report_results_to_ide: true,
            tests_must_run_on_main_thread: true,
            test_timeouts_enabled: false,
            additional_fields,
        }
    }

    pub fn launch_environment(
        &self,
        product_major_version: u64,
        session_identifier: Uuid,
    ) -> HashMap<String, String> {
        let mut env = HashMap::from([
            (
                "CA_ASSERT_MAIN_THREAD_TRANSACTIONS".to_string(),
                "0".to_string(),
            ),
            ("CA_DEBUG_TRANSACTIONS".to_string(), "0".to_string()),
            (
                "DYLD_FRAMEWORK_PATH".to_string(),
                format!("{}/Frameworks:", self.runner.path),
            ),
            (
                "DYLD_LIBRARY_PATH".to_string(),
                format!("{}/Frameworks", self.runner.path),
            ),
            ("MTC_CRASH_ON_REPORT".to_string(), "1".to_string()),
            ("NSUnbufferedIO".to_string(), "YES".to_string()),
            (
                "SQLITE_ENABLE_THREAD_ASSERTIONS".to_string(),
                "1".to_string(),
            ),
            ("WDA_PRODUCT_BUNDLE_IDENTIFIER".to_string(), String::new()),
            ("XCTestBundlePath".to_string(), self.test_bundle_path()),
            (
                "XCTestSessionIdentifier".to_string(),
                session_identifier.to_string().to_uppercase(),
            ),
            (
                "XCODE_DBG_XPC_EXCLUSIONS".to_string(),
                "com.apple.dt.xctestSymbolicator".to_string(),
            ),
        ]);

        if let Some(container) = &self.runner.container {
            env.insert(
                "XCTestConfigurationFilePath".to_string(),
                format!(
                    "{container}/tmp/{}.xctestconfiguration",
                    session_identifier.to_string().to_uppercase()
                ),
            );
        }
        if product_major_version >= 11 {
            env.insert(
                "DYLD_INSERT_LIBRARIES".to_string(),
                "/Developer/usr/lib/libMainThreadChecker.dylib".to_string(),
            );
            env.insert("OS_ACTIVITY_DT_MODE".to_string(), "YES".to_string());
        }
        if product_major_version >= 17 {
            env.insert(
                "DYLD_FRAMEWORK_PATH".to_string(),
                format!(
                    "{}/Frameworks:/System/Developer/Library/Frameworks:",
                    self.runner.path
                ),
            );
            env.insert(
                "DYLD_LIBRARY_PATH".to_string(),
                format!("{}/Frameworks:/System/Developer/usr/lib", self.runner.path),
            );
            // iOS 17+ uses DDI path; clear the container-based config file path set above.
            env.insert("XCTestConfigurationFilePath".to_string(), String::new());
            env.insert("XCTestManagerVariant".to_string(), "DDI".to_string());
        }

        for (key, value) in &self.env {
            if is_internal_target_app_key(key) {
                continue;
            }
            env.insert(key.clone(), value.clone());
        }
        env
    }

    pub fn launch_arguments(&self) -> Vec<String> {
        let mut args = vec![
            "-NSTreatUnknownArgumentsAsOpen".to_string(),
            "NO".to_string(),
            "-ApplePersistenceIgnoreState".to_string(),
            "YES".to_string(),
        ];
        args.extend(self.args.clone());
        args
    }

    pub fn launch_options(&self, product_major_version: u64) -> Vec<(String, Value)> {
        let mut options = vec![("StartSuspendedKey".to_string(), Value::Boolean(false))];
        if product_major_version >= 12 {
            options.push(("ActivateSuspended".to_string(), Value::Boolean(true)));
        }
        options
    }

    fn target_application_arguments(&self) -> Vec<String> {
        self.env
            .get(TARGET_APP_ARGS_KEY)
            .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
            .unwrap_or_default()
    }

    fn target_application_environment(&self) -> plist::Dictionary {
        self.env
            .get(TARGET_APP_ENV_KEY)
            .and_then(|value| serde_json::from_str::<HashMap<String, String>>(value).ok())
            .map(|env| {
                env.into_iter()
                    .map(|(key, value)| (key, Value::String(value)))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn store_target_app_context(
    dst: &mut HashMap<String, String>,
    env: &HashMap<String, Value>,
    args: &[String],
) {
    let mut target_env = HashMap::new();
    merge_string_values(&mut target_env, env);
    if !target_env.is_empty() {
        // Safety: serde_json::to_string on HashMap<String, String> is infallible
        // (no non-string keys, no recursive structures, no unsupported types).
        dst.insert(
            TARGET_APP_ENV_KEY.to_string(),
            serde_json::to_string(&target_env).unwrap(),
        );
    }
    if !args.is_empty() {
        // Safety: serde_json::to_string on &[String] is infallible.
        dst.insert(
            TARGET_APP_ARGS_KEY.to_string(),
            serde_json::to_string(args).unwrap(),
        );
    }
}

fn merge_string_values(dst: &mut HashMap<String, String>, src: &HashMap<String, Value>) {
    for (key, value) in src {
        if let Some(value) = value_as_string(value) {
            dst.insert(key.clone(), value);
        }
    }
}

fn value_as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Boolean(flag) => Some(if *flag { "true" } else { "false" }.to_string()),
        Value::Integer(n) => Some(n.to_string()),
        Value::Real(n) => Some(n.to_string()),
        _ => None,
    }
}

fn is_internal_target_app_key(key: &str) -> bool {
    matches!(key, TARGET_APP_ENV_KEY | TARGET_APP_ARGS_KEY)
}

fn bundle_name_from_path(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

fn product_module_name(xctest_bundle_name: &str) -> String {
    xctest_bundle_name.trim_end_matches(".xctest").to_string()
}

fn default_capabilities() -> XctCapabilities {
    XctCapabilities {
        capabilities: vec![
            (
                "expected failure test capability".to_string(),
                Value::Boolean(true),
            ),
            (
                "test case run configurations".to_string(),
                Value::Boolean(true),
            ),
            ("test timeout capability".to_string(), Value::Boolean(true)),
            ("test iterations".to_string(), Value::Boolean(true)),
            (
                "request diagnostics for specific devices".to_string(),
                Value::Boolean(true),
            ),
            (
                "delayed attachment transfer".to_string(),
                Value::Boolean(true),
            ),
            ("skipped test capability".to_string(), Value::Boolean(true)),
            (
                "daemon container sandbox extension".to_string(),
                Value::Boolean(true),
            ),
            (
                "ubiquitous test identifiers".to_string(),
                Value::Boolean(true),
            ),
            ("XCTIssue capability".to_string(), Value::Boolean(true)),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner() -> InstalledAppInfo {
        InstalledAppInfo {
            bundle_id: "com.example.Runner".to_string(),
            path: "/private/var/containers/Bundle/Application/Runner.app".to_string(),
            executable: "DemoAppUITests-Runner".to_string(),
            container: Some("/private/var/mobile/Containers/Data/Application/Runner".to_string()),
        }
    }

    #[test]
    fn launch_environment_uses_ddi_variant_on_ios17() {
        let plan = TestLaunchPlan {
            runner: runner(),
            target: None,
            xctest_bundle_name: "DemoAppUITests.xctest".to_string(),
            is_xctest: false,
            args: Vec::new(),
            env: HashMap::new(),
            tests_to_run: Vec::new(),
            tests_to_skip: Vec::new(),
        };

        let env = plan.launch_environment(
            17,
            Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap(),
        );
        assert_eq!(
            env.get("XCTestManagerVariant").map(String::as_str),
            Some("DDI")
        );
        assert_eq!(
            env.get("XCTestConfigurationFilePath").map(String::as_str),
            Some("")
        );
    }

    #[test]
    fn from_scheme_preserves_target_app_context_without_changing_runner_env_behavior() {
        let scheme = SchemeData {
            test_host_bundle_identifier: "com.example.Runner".to_string(),
            test_bundle_path: "DemoAppUITests.xctest".to_string(),
            skip_test_identifiers: Vec::new(),
            only_test_identifiers: vec!["DemoAppUITests/LoginTests/testHappyPath".to_string()],
            is_ui_test_bundle: true,
            command_line_arguments: vec!["-RunnerFlag".to_string()],
            environment_variables: HashMap::from([(
                "RUNNER_ENV".to_string(),
                Value::String("runner".to_string()),
            )]),
            testing_environment_variables: HashMap::new(),
            ui_target_app_environment_variables: HashMap::from([(
                "TARGET_ENV".to_string(),
                Value::String("target".to_string()),
            )]),
            ui_target_app_command_line_arguments: vec![
                "-AppleLanguages".to_string(),
                "(en)".to_string(),
            ],
            ui_target_app_path: "__TESTROOT__/Debug-iphoneos/DemoApp.app".to_string(),
        };
        let plan = TestLaunchPlan::from_scheme(
            &scheme,
            runner(),
            Some(InstalledAppInfo {
                bundle_id: "com.example.Target".to_string(),
                path: "/private/var/containers/Bundle/Application/Target.app".to_string(),
                executable: "DemoApp".to_string(),
                container: None,
            }),
        );

        let launch_env = plan.launch_environment(
            17,
            Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap(),
        );
        assert_eq!(
            launch_env.get("RUNNER_ENV").map(String::as_str),
            Some("runner")
        );
        assert_eq!(
            launch_env.get("TARGET_ENV").map(String::as_str),
            Some("target")
        );
        assert!(!launch_env.contains_key(TARGET_APP_ENV_KEY));
        assert!(!launch_env.contains_key(TARGET_APP_ARGS_KEY));
    }

    #[test]
    fn configuration_adds_target_application_fields_for_ui_tests() {
        let mut env = HashMap::new();
        store_target_app_context(
            &mut env,
            &HashMap::from([(
                "TARGET_ENV".to_string(),
                Value::String("target".to_string()),
            )]),
            &["-AppleLanguages".to_string(), "(en)".to_string()],
        );
        let plan = TestLaunchPlan {
            runner: runner(),
            target: Some(InstalledAppInfo {
                bundle_id: "com.example.Target".to_string(),
                path: "/private/var/containers/Bundle/Application/Target.app".to_string(),
                executable: "DemoApp".to_string(),
                container: None,
            }),
            xctest_bundle_name: "DemoAppUITests.xctest".to_string(),
            is_xctest: false,
            args: Vec::new(),
            env,
            tests_to_run: vec!["DemoAppUITests/LoginTests/testHappyPath".to_string()],
            tests_to_skip: Vec::new(),
        };

        let config = plan.xctest_configuration(
            17,
            Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap(),
        );
        assert!(config
            .additional_fields
            .iter()
            .any(|(key, _)| key == "targetApplicationBundleID"));
        assert!(config
            .additional_fields
            .iter()
            .any(|(key, _)| key == "testsToRun"));
        assert!(config.additional_fields.iter().any(|(key, value)| {
            key == "targetApplicationArguments"
                && matches!(
                    value,
                    Value::Array(items)
                        if items
                            == &vec![
                                Value::String("-AppleLanguages".to_string()),
                                Value::String("(en)".to_string()),
                            ]
                )
        }));
        assert!(config.additional_fields.iter().any(|(key, value)| {
            key == "targetApplicationEnvironment"
                && matches!(
                    value,
                    Value::Dictionary(items)
                        if items.get("TARGET_ENV") == Some(&Value::String("target".to_string()))
                )
        }));
    }
}
