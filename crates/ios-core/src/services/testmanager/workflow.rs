use std::collections::{HashMap, HashSet};

use crate::proto::nskeyedarchiver_encode::{NsUrl, XcTestConfiguration, XctCapabilities};
use plist::{Dictionary, Uid, Value};
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

/// A direct XCTest invocation, independent of an .xctestrun file.
///
/// The plan deliberately keeps bundle identifiers and selectors as strings:
/// XCTest accepts Unicode names and the device is the authority for whether a
/// particular identifier exists. Structural validation is done by the builder
/// so malformed paths/control characters cannot accidentally become a bundle
/// or test-bundle path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunXcTestPlan {
    /// Optional application-under-test bundle identifier. Unit tests do not
    /// need one; UI tests normally do.
    pub bundle_id: Option<String>,
    /// Installed XCTest runner application bundle identifier.
    pub test_runner_bundle_id: String,
    /// Test bundle name inside the runner, normally FooTests.xctest.
    pub xctest_config: String,
    /// XCTest selectors to execute, in the spelling accepted by testmanagerd.
    pub tests_to_run: Vec<String>,
    /// XCTest selectors to skip.
    pub tests_to_skip: Vec<String>,
    /// Arguments passed to the runner process.
    pub args: Vec<String>,
    /// Environment passed to the runner process.
    pub env: HashMap<String, String>,
    /// Whether this is a unit-test bundle (true) rather than a UI-test bundle
    /// (false).
    pub is_xctest: bool,
}

/// Validation failures for a direct XCTest plan.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunXcTestPlanError {
    #[error("test runner bundle identifier is required")]
    MissingRunnerBundleId,
    #[error("xctest config name is required")]
    MissingXcTestConfig,
    #[error("xctest config name must be a single .xctest bundle component: {0:?}")]
    InvalidXcTestConfig(String),
    #[error("{field} must not be empty")]
    EmptyField { field: &'static str },
    #[error("{field} contains a NUL or control character")]
    InvalidText { field: &'static str },
    #[error("xctest {field} must be a single component without '/' or '\\\\': {value:?}")]
    InvalidComponent { field: &'static str, value: String },
    #[error("--method requires --class")]
    MethodWithoutClass,
    #[error("--test cannot be combined with --class or --method")]
    ConflictingSelection,
    #[error("duplicate {field}: {value:?}")]
    DuplicateValue { field: &'static str, value: String },
    #[error("environment variable name must not be empty")]
    EmptyEnvironmentName,
    #[error("environment variable name contains '=' or a NUL/control character: {0:?}")]
    InvalidEnvironmentName(String),
}

/// Builder for RunXcTestPlan.
///
/// --test-style selectors and the class/method convenience pair are
/// intentionally mutually exclusive. This keeps the resulting XCTest
/// configuration deterministic and catches accidental combinations before a
/// device connection is opened.
#[derive(Debug, Clone, Default)]
pub struct RunXcTestPlanBuilder {
    bundle_id: Option<String>,
    test_runner_bundle_id: Option<String>,
    xctest_config: Option<String>,
    tests_to_run: Vec<String>,
    tests_to_skip: Vec<String>,
    class: Option<String>,
    method: Option<String>,
    args: Vec<String>,
    env_entries: Vec<(String, String)>,
    is_xctest: bool,
}

impl RunXcTestPlan {
    /// Start a direct-runner plan with the required runner and test-bundle
    /// names.
    pub fn builder(
        test_runner_bundle_id: impl Into<String>,
        xctest_config: impl Into<String>,
    ) -> RunXcTestPlanBuilder {
        RunXcTestPlanBuilder::new(test_runner_bundle_id, xctest_config)
    }

    /// Convert this device-independent plan into the existing testmanager
    /// launch representation after installed app metadata has been resolved.
    pub fn into_test_launch_plan(
        self,
        runner: InstalledAppInfo,
        target: Option<InstalledAppInfo>,
    ) -> TestLaunchPlan {
        TestLaunchPlan {
            runner,
            target,
            xctest_bundle_name: self.xctest_config,
            is_xctest: self.is_xctest,
            args: self.args,
            env: self.env,
            tests_to_run: self.tests_to_run,
            tests_to_skip: self.tests_to_skip,
        }
    }
}

impl RunXcTestPlanBuilder {
    pub fn new(test_runner_bundle_id: impl Into<String>, xctest_config: impl Into<String>) -> Self {
        Self {
            test_runner_bundle_id: Some(test_runner_bundle_id.into()),
            xctest_config: Some(xctest_config.into()),
            ..Self::default()
        }
    }

    pub fn bundle_id(mut self, bundle_id: impl Into<String>) -> Self {
        self.bundle_id = Some(bundle_id.into());
        self
    }

    /// Alias useful to callers that use the same terminology as the CLI.
    pub fn target_bundle_id(self, bundle_id: impl Into<String>) -> Self {
        self.bundle_id(bundle_id)
    }

    pub fn test_runner_bundle_id(mut self, bundle_id: impl Into<String>) -> Self {
        self.test_runner_bundle_id = Some(bundle_id.into());
        self
    }

    pub fn xctest_config(mut self, config: impl Into<String>) -> Self {
        self.xctest_config = Some(config.into());
        self
    }

    pub fn test(mut self, selector: impl Into<String>) -> Self {
        self.tests_to_run.push(selector.into());
        self
    }

    pub fn tests_to_run<I, S>(mut self, selectors: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tests_to_run
            .extend(selectors.into_iter().map(Into::into));
        self
    }

    pub fn skip(mut self, selector: impl Into<String>) -> Self {
        self.tests_to_skip.push(selector.into());
        self
    }

    pub fn tests_to_skip<I, S>(mut self, selectors: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tests_to_skip
            .extend(selectors.into_iter().map(Into::into));
        self
    }

    pub fn class(mut self, class: impl Into<String>) -> Self {
        self.class = Some(class.into());
        self
    }

    pub fn method(mut self, method: impl Into<String>) -> Self {
        self.method = Some(method.into());
        self
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_entries.push((key.into(), value.into()));
        self
    }

    pub fn environment_variable(self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env(key, value)
    }

    pub fn xctest(mut self, is_xctest: bool) -> Self {
        self.is_xctest = is_xctest;
        self
    }

    pub fn build(self) -> Result<RunXcTestPlan, RunXcTestPlanError> {
        let test_runner_bundle_id = self
            .test_runner_bundle_id
            .ok_or(RunXcTestPlanError::MissingRunnerBundleId)?;
        if test_runner_bundle_id.is_empty() {
            return Err(RunXcTestPlanError::MissingRunnerBundleId);
        }
        validate_text("test runner bundle identifier", &test_runner_bundle_id)?;

        let xctest_config = self
            .xctest_config
            .ok_or(RunXcTestPlanError::MissingXcTestConfig)?;
        if xctest_config.is_empty() {
            return Err(RunXcTestPlanError::MissingXcTestConfig);
        }
        validate_component("xctest config name", &xctest_config)?;
        if !xctest_config.ends_with(".xctest") {
            return Err(RunXcTestPlanError::InvalidXcTestConfig(xctest_config));
        }

        let bundle_id = match self.bundle_id {
            Some(bundle_id) => {
                if bundle_id.is_empty() {
                    return Err(RunXcTestPlanError::EmptyField {
                        field: "application bundle identifier",
                    });
                }
                validate_text("application bundle identifier", &bundle_id)?;
                Some(bundle_id)
            }
            None => None,
        };

        if self.method.is_some() && self.class.is_none() {
            return Err(RunXcTestPlanError::MethodWithoutClass);
        }
        if !self.tests_to_run.is_empty() && (self.class.is_some() || self.method.is_some()) {
            return Err(RunXcTestPlanError::ConflictingSelection);
        }

        let mut tests_to_run = self.tests_to_run;
        if let Some(class) = self.class {
            validate_component("test class", &class)?;
            let selector = match self.method {
                Some(method) => {
                    validate_component("test method", &method)?;
                    format!("{class}/{method}")
                }
                None => class,
            };
            tests_to_run.push(selector);
        }
        validate_unique_selectors("test selector", &tests_to_run)?;
        validate_unique_selectors("skip selector", &self.tests_to_skip)?;

        for arg in &self.args {
            validate_text("runner argument", arg)?;
        }

        let mut env = HashMap::with_capacity(self.env_entries.len());
        for (key, value) in self.env_entries {
            if key.is_empty() {
                return Err(RunXcTestPlanError::EmptyEnvironmentName);
            }
            if key.contains('=') {
                return Err(RunXcTestPlanError::InvalidEnvironmentName(key));
            }
            validate_text("environment variable name", &key)?;
            validate_text("environment variable value", &value)?;
            if env.insert(key.clone(), value).is_some() {
                return Err(RunXcTestPlanError::DuplicateValue {
                    field: "environment variable",
                    value: key,
                });
            }
        }

        Ok(RunXcTestPlan {
            bundle_id,
            test_runner_bundle_id,
            xctest_config,
            tests_to_run,
            tests_to_skip: self.tests_to_skip,
            args: self.args,
            env,
            is_xctest: self.is_xctest,
        })
    }
}

fn validate_text(field: &'static str, value: &str) -> Result<(), RunXcTestPlanError> {
    if value
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        Err(RunXcTestPlanError::InvalidText { field })
    } else {
        Ok(())
    }
}

fn validate_component(field: &'static str, value: &str) -> Result<(), RunXcTestPlanError> {
    if value.is_empty() {
        return Err(RunXcTestPlanError::EmptyField { field });
    }
    validate_text(field, value)?;
    if value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.starts_with(':')
    {
        return Err(RunXcTestPlanError::InvalidComponent {
            field,
            value: value.to_string(),
        });
    }
    Ok(())
}

fn validate_unique_selectors(
    field: &'static str,
    selectors: &[String],
) -> Result<(), RunXcTestPlanError> {
    let mut seen = HashSet::with_capacity(selectors.len());
    for selector in selectors {
        validate_selector(field, selector)?;
        if !seen.insert(selector) {
            return Err(RunXcTestPlanError::DuplicateValue {
                field,
                value: selector.clone(),
            });
        }
    }
    Ok(())
}

fn validate_selector(field: &'static str, selector: &str) -> Result<(), RunXcTestPlanError> {
    validate_text(field, selector)?;
    if selector.is_empty() {
        return Err(RunXcTestPlanError::EmptyField { field });
    }
    let mut components = selector.split('/');
    let class = components
        .next()
        .ok_or(RunXcTestPlanError::EmptyField { field })?;
    validate_component("test class", class)?;
    if let Some(method) = components.next() {
        validate_component("test method", method)?;
    }
    if components.next().is_some() {
        return Err(RunXcTestPlanError::InvalidComponent {
            field,
            value: selector.to_string(),
        });
    }
    Ok(())
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

        let mut additional_fields = reference_default_xctest_fields();

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
                // iOS 12+ resolves this URL relative to the installed runner
                // bundle (the same in-memory configuration used by go-ios).
                // iOS 11 still consumes the absolute path from its on-device
                // configuration-file workflow.
                path: if product_major_version >= 12 {
                    format!("PlugIns/{}", self.xctest_bundle_name)
                } else {
                    self.test_bundle_path()
                },
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
                if product_major_version >= 17 {
                    session_identifier.to_string().to_uppercase()
                } else {
                    session_identifier.to_string()
                },
            ),
            (
                "XCODE_DBG_XPC_EXCLUSIONS".to_string(),
                "com.apple.dt.xctestSymbolicator".to_string(),
            ),
        ]);

        if product_major_version < 14 {
            if let Some(container) = &self.runner.container {
                env.insert(
                    "XCTestConfigurationFilePath".to_string(),
                    format!("{container}/tmp/{}.xctestconfiguration", session_identifier),
                );
            }
        } else {
            env.insert("XCTestConfigurationFilePath".to_string(), String::new());
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
            // iOS 17+ uses the DDI path and the in-memory testmanager
            // configuration; the config file path remains empty.
            env.insert("XCTestManagerVariant".to_string(), "DDI".to_string());
            if self.is_xctest {
                env.insert(
                    "DYLD_INSERT_LIBRARIES".to_string(),
                    "/Developer/usr/lib/libMainThreadChecker.dylib:/System/Developer/usr/lib/libXCTestBundleInject.dylib"
                        .to_string(),
                );
            }
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
        if product_major_version >= 17 && !self.is_xctest {
            options.push(("__ActivateSuspended".to_string(), Value::Boolean(true)));
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

fn reference_default_xctest_fields() -> Vec<(String, Value)> {
    vec![
        (
            "aggregateStatisticsBeforeCrash".to_string(),
            Value::Dictionary(Dictionary::from_iter([(
                "XCSuiteRecordsKey".to_string(),
                Value::Dictionary(Dictionary::new()),
            )])),
        ),
        ("baselineFileRelativePath".to_string(), ns_null()),
        ("baselineFileURL".to_string(), ns_null()),
        ("defaultTestExecutionTimeAllowance".to_string(), ns_null()),
        (
            "disablePerformanceMetrics".to_string(),
            Value::Boolean(false),
        ),
        ("emitOSLogs".to_string(), Value::Boolean(false)),
        (
            "gatherLocalizableStringsData".to_string(),
            Value::Boolean(false),
        ),
        ("maximumTestExecutionTimeAllowance".to_string(), ns_null()),
        ("randomExecutionOrderingSeed".to_string(), ns_null()),
        ("reportActivities".to_string(), Value::Boolean(true)),
        (
            "systemAttachmentLifetime".to_string(),
            Value::Integer(2.into()),
        ),
        (
            "testApplicationDependencies".to_string(),
            Value::Dictionary(Dictionary::new()),
        ),
        ("testApplicationUserOverrides".to_string(), ns_null()),
        ("testBundleRelativePath".to_string(), ns_null()),
        (
            "testExecutionOrdering".to_string(),
            Value::Integer(0.into()),
        ),
        ("testsDrivenByIDE".to_string(), Value::Boolean(false)),
        (
            "treatMissingBaselinesAsFailures".to_string(),
            Value::Boolean(false),
        ),
        (
            "userAttachmentLifetime".to_string(),
            Value::Integer(0.into()),
        ),
        (
            "preferredScreenCaptureFormat".to_string(),
            Value::Integer(2.into()),
        ),
    ]
}

fn ns_null() -> Value {
    Value::Uid(Uid::new(0))
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
    fn launch_environment_uses_in_memory_configuration_on_classic_ios14() {
        let plan = TestLaunchPlan {
            runner: runner(),
            target: None,
            xctest_bundle_name: "DemoTests.xctest".to_string(),
            is_xctest: false,
            args: Vec::new(),
            env: HashMap::new(),
            tests_to_run: Vec::new(),
            tests_to_skip: Vec::new(),
        };

        let env = plan.launch_environment(
            14,
            Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap(),
        );
        assert_eq!(
            env.get("XCTestConfigurationFilePath").map(String::as_str),
            Some("")
        );
        assert!(!env.contains_key("XCTestManagerVariant"));
        assert_eq!(
            env.get("XCTestSessionIdentifier").map(String::as_str),
            Some("00112233-4455-6677-8899-aabbccddeeff")
        );
    }

    #[test]
    fn launch_environment_uses_legacy_container_configuration_on_ios13() {
        let plan = TestLaunchPlan {
            runner: runner(),
            target: None,
            xctest_bundle_name: "DemoTests.xctest".to_string(),
            is_xctest: true,
            args: Vec::new(),
            env: HashMap::new(),
            tests_to_run: Vec::new(),
            tests_to_skip: Vec::new(),
        };

        let env = plan.launch_environment(
            13,
            Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap(),
        );
        assert_eq!(
            env.get("XCTestConfigurationFilePath").map(String::as_str),
            Some(
                "/private/var/mobile/Containers/Data/Application/Runner/tmp/00112233-4455-6677-8899-aabbccddeeff.xctestconfiguration"
            )
        );
        assert_eq!(
            env.get("XCTestSessionIdentifier").map(String::as_str),
            Some("00112233-4455-6677-8899-aabbccddeeff")
        );
    }

    #[test]
    fn launch_environment_injects_xctest_bundle_library_for_unit_tests_on_ios17() {
        let plan = TestLaunchPlan {
            runner: runner(),
            target: None,
            xctest_bundle_name: "DemoTests.xctest".to_string(),
            is_xctest: true,
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
            env.get("DYLD_INSERT_LIBRARIES").map(String::as_str),
            Some(
                "/Developer/usr/lib/libMainThreadChecker.dylib:/System/Developer/usr/lib/libXCTestBundleInject.dylib"
            )
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
        assert_eq!(config.test_bundle_url.path, "PlugIns/DemoAppUITests.xctest");
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

    #[test]
    fn configuration_uses_relative_bundle_url_since_ios12_and_absolute_on_ios11() {
        let plan = TestLaunchPlan {
            runner: runner(),
            target: None,
            xctest_bundle_name: "DemoTests.xctest".to_string(),
            is_xctest: true,
            args: Vec::new(),
            env: HashMap::new(),
            tests_to_run: Vec::new(),
            tests_to_skip: Vec::new(),
        };

        assert_eq!(
            plan.xctest_configuration(
                16,
                Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap()
            )
            .test_bundle_url
            .path,
            "PlugIns/DemoTests.xctest"
        );
        assert_eq!(
            plan.xctest_configuration(
                11,
                Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap()
            )
            .test_bundle_url
            .path,
            "/private/var/containers/Bundle/Application/Runner.app/PlugIns/DemoTests.xctest"
        );
    }

    #[test]
    fn configuration_includes_reference_default_fields() {
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

        let config = plan.xctest_configuration(
            17,
            Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap(),
        );

        assert!(config.additional_fields.iter().any(|(key, value)| {
            key == "aggregateStatisticsBeforeCrash"
                && matches!(
                    value,
                    Value::Dictionary(stats)
                        if matches!(
                            stats.get("XCSuiteRecordsKey"),
                            Some(Value::Dictionary(suites)) if suites.is_empty()
                        )
                )
        }));
        assert!(config.additional_fields.iter().any(|(key, value)| {
            key == "disablePerformanceMetrics" && value.as_boolean() == Some(false)
        }));
        assert!(config.additional_fields.iter().any(|(key, value)| {
            key == "systemAttachmentLifetime" && value.as_signed_integer() == Some(2)
        }));
        assert!(config.additional_fields.iter().any(|(key, value)| {
            key == "preferredScreenCaptureFormat" && value.as_signed_integer() == Some(2)
        }));
        assert!(config.additional_fields.iter().any(|(key, value)| {
            key == "testsDrivenByIDE" && value.as_boolean() == Some(false)
        }));
    }

    #[test]
    fn direct_plan_builder_supports_unicode_selection_environment_and_arguments() {
        let plan = RunXcTestPlan::builder("com.example.Runner", "DemoTests.xctest")
            .bundle_id("com.example.App")
            .test("模块.LoginTests/test通过")
            .skip("模块.FlakyTests")
            .env("ключ", "значение=✓")
            .arg("--название")
            .xctest(false)
            .build()
            .unwrap();

        assert_eq!(plan.bundle_id.as_deref(), Some("com.example.App"));
        assert_eq!(plan.tests_to_run, ["模块.LoginTests/test通过"]);
        assert_eq!(plan.tests_to_skip, ["模块.FlakyTests"]);
        assert_eq!(plan.env.get("ключ").map(String::as_str), Some("значение=✓"));
        assert_eq!(plan.args, ["--название"]);
        assert!(!plan.is_xctest);
    }

    #[test]
    fn direct_plan_builder_supports_class_and_method_selection() {
        let plan = RunXcTestPlan::builder("com.example.Runner", "DemoTests.xctest")
            .class("LoginTests")
            .method("testHappyPath")
            .build()
            .unwrap();

        assert_eq!(plan.tests_to_run, ["LoginTests/testHappyPath"]);
    }

    #[test]
    fn direct_plan_builder_rejects_ambiguous_or_unsafe_inputs() {
        assert_eq!(
            RunXcTestPlan::builder("com.example.Runner", "DemoTests.xctest")
                .method("test")
                .build()
                .unwrap_err(),
            RunXcTestPlanError::MethodWithoutClass
        );
        assert_eq!(
            RunXcTestPlan::builder("com.example.Runner", "DemoTests.xctest")
                .class("LoginTests")
                .test("OtherTests/test")
                .build()
                .unwrap_err(),
            RunXcTestPlanError::ConflictingSelection
        );
        assert!(matches!(
            RunXcTestPlan::builder("com.example.Runner", "../DemoTests.xctest").build(),
            Err(RunXcTestPlanError::InvalidComponent { .. })
        ));
        assert!(matches!(
            RunXcTestPlan::builder("com.example.Runner", "DemoTests.xctest")
                .env("A=B", "value")
                .build(),
            Err(RunXcTestPlanError::InvalidEnvironmentName(_))
        ));
        assert!(matches!(
            RunXcTestPlan::builder("com.example.Runner", "DemoTests.xctest")
                .test("LoginTests/")
                .build(),
            Err(RunXcTestPlanError::EmptyField { .. })
        ));
        assert!(matches!(
            RunXcTestPlan::builder("com.example.Runner", "DemoTests.xctest")
                .test("LoginTests/test/extra")
                .build(),
            Err(RunXcTestPlanError::InvalidComponent { .. })
        ));
    }

    #[test]
    fn direct_plan_converts_to_existing_launch_plan() {
        let plan = RunXcTestPlan::builder("com.example.Runner", "DemoTests.xctest")
            .bundle_id("com.example.App")
            .xctest(true)
            .build()
            .unwrap();
        let launch = plan.into_test_launch_plan(runner(), None);

        assert_eq!(launch.runner.bundle_id, "com.example.Runner");
        assert_eq!(launch.xctest_bundle_name, "DemoTests.xctest");
        assert!(launch.is_xctest);
    }
}
