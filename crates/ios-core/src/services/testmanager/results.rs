use crate::services::dtx::{DtxMessage, DtxPayload, NSObject};
use serde::Serialize;

pub const DID_BEGIN_EXECUTING_TEST_PLAN_SELECTOR: &str = "_XCT_didBeginExecutingTestPlan";
pub const DID_FINISH_EXECUTING_TEST_PLAN_SELECTOR: &str = "_XCT_didFinishExecutingTestPlan";
pub const LOG_MESSAGE_SELECTOR: &str = "_XCT_logMessage:";
pub const LOG_DEBUG_MESSAGE_SELECTOR: &str = "_XCT_logDebugMessage:";
pub const TEST_SUITE_STARTED_SELECTOR: &str = "_XCT_testSuite:didStartAt:";
pub const TEST_SUITE_FINISHED_SELECTOR: &str =
    "_XCT_testSuite:didFinishAt:runCount:withFailures:unexpected:testDuration:totalDuration:";
pub const TEST_SUITE_FINISHED_WITH_SKIP_SELECTOR: &str =
    "_XCT_testSuiteWithIdentifier:didFinishAt:runCount:skipCount:failureCount:expectedFailureCount:uncaughtExceptionCount:testDuration:totalDuration:";
pub const TEST_CASE_STARTED_SELECTOR: &str = "_XCT_testCaseDidStartForTestClass:method:";
pub const TEST_CASE_FINISHED_SELECTOR: &str =
    "_XCT_testCaseDidFinishForTestClass:method:withStatus:duration:";
pub const TEST_CASE_FAILED_SELECTOR: &str =
    "_XCT_testCaseDidFailForTestClass:method:withMessage:file:line:";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TestExecutionEvent {
    BeganPlan,
    FinishedPlan,
    Log {
        message: String,
        debug: bool,
    },
    SuiteStarted {
        name: String,
        started_at: Option<String>,
    },
    SuiteFinished {
        name: String,
        finished_at: Option<String>,
        test_count: u64,
        skipped: u64,
        failures: u64,
        expected_failures: u64,
        unexpected_failures: u64,
        uncaught_exceptions: u64,
        test_duration_seconds: f64,
        total_duration_seconds: f64,
    },
    CaseStarted {
        class_name: String,
        method_name: String,
    },
    CaseFailed {
        class_name: String,
        method_name: String,
        message: String,
        file: Option<String>,
        line: Option<u64>,
    },
    CaseFinished {
        class_name: String,
        method_name: String,
        status: TestCaseStatus,
        duration_seconds: f64,
    },
}

impl TestExecutionEvent {
    pub fn from_dtx_message(message: &DtxMessage) -> Option<Self> {
        let DtxPayload::MethodInvocation { selector, args } = &message.payload else {
            return None;
        };
        match selector.as_str() {
            DID_BEGIN_EXECUTING_TEST_PLAN_SELECTOR => Some(Self::BeganPlan),
            DID_FINISH_EXECUTING_TEST_PLAN_SELECTOR => Some(Self::FinishedPlan),
            LOG_MESSAGE_SELECTOR => Some(Self::Log {
                message: string_arg(args, 0)?,
                debug: false,
            }),
            LOG_DEBUG_MESSAGE_SELECTOR => Some(Self::Log {
                message: string_arg(args, 0)?,
                debug: true,
            }),
            TEST_SUITE_STARTED_SELECTOR => Some(Self::SuiteStarted {
                name: string_arg(args, 0)?,
                started_at: optional_string_arg(args, 1),
            }),
            TEST_SUITE_FINISHED_SELECTOR => Some(Self::SuiteFinished {
                name: string_arg(args, 0)?,
                finished_at: optional_string_arg(args, 1),
                test_count: uint_arg(args, 2).unwrap_or(0),
                skipped: 0,
                failures: uint_arg(args, 3).unwrap_or(0),
                expected_failures: 0,
                unexpected_failures: uint_arg(args, 4).unwrap_or(0),
                uncaught_exceptions: 0,
                test_duration_seconds: double_arg(args, 5).unwrap_or(0.0),
                total_duration_seconds: double_arg(args, 6).unwrap_or(0.0),
            }),
            TEST_SUITE_FINISHED_WITH_SKIP_SELECTOR => {
                let name = identifier_suite_name(args.first())?;
                Some(Self::SuiteFinished {
                    name,
                    finished_at: optional_string_arg(args, 1),
                    test_count: uint_arg(args, 2).unwrap_or(0),
                    skipped: uint_arg(args, 3).unwrap_or(0),
                    failures: uint_arg(args, 4).unwrap_or(0),
                    expected_failures: uint_arg(args, 5).unwrap_or(0),
                    unexpected_failures: 0,
                    uncaught_exceptions: uint_arg(args, 6).unwrap_or(0),
                    test_duration_seconds: double_arg(args, 7).unwrap_or(0.0),
                    total_duration_seconds: double_arg(args, 8).unwrap_or(0.0),
                })
            }
            TEST_CASE_STARTED_SELECTOR => Some(Self::CaseStarted {
                class_name: string_arg(args, 0)?,
                method_name: string_arg(args, 1)?,
            }),
            TEST_CASE_FAILED_SELECTOR => Some(Self::CaseFailed {
                class_name: string_arg(args, 0)?,
                method_name: string_arg(args, 1)?,
                message: string_arg(args, 2).unwrap_or_default(),
                file: optional_string_arg(args, 3),
                line: uint_arg(args, 4),
            }),
            TEST_CASE_FINISHED_SELECTOR => Some(Self::CaseFinished {
                class_name: string_arg(args, 0)?,
                method_name: string_arg(args, 1)?,
                status: TestCaseStatus::from_wda_status(&string_arg(args, 2)?),
                duration_seconds: double_arg(args, 3).unwrap_or(0.0),
            }),
            _ => None,
        }
    }

    pub fn is_finished_plan(&self) -> bool {
        matches!(self, Self::FinishedPlan)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TestCaseStatus {
    Passed,
    Failed,
    ExpectedFailure,
    Stalled,
    Skipped,
    Other(String),
}

impl TestCaseStatus {
    fn from_wda_status(status: &str) -> Self {
        match status {
            "passed" => Self::Passed,
            "failed" => Self::Failed,
            "expected failure" => Self::ExpectedFailure,
            "stalled" => Self::Stalled,
            "skipped" => Self::Skipped,
            other => Self::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestFailure {
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestCaseSummary {
    pub class_name: String,
    pub method_name: String,
    pub status: Option<TestCaseStatus>,
    pub duration_seconds: Option<f64>,
    pub failure: Option<TestFailure>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestSuiteSummary {
    pub name: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub test_count: Option<u64>,
    pub skipped: Option<u64>,
    pub failures: Option<u64>,
    pub expected_failures: Option<u64>,
    pub unexpected_failures: Option<u64>,
    pub uncaught_exceptions: Option<u64>,
    pub test_duration_seconds: Option<f64>,
    pub total_duration_seconds: Option<f64>,
    pub cases: Vec<TestCaseSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestRunSummary {
    pub began: bool,
    pub finished: bool,
    pub total_tests: u64,
    pub failed_tests: u64,
    pub skipped_tests: u64,
    pub logs: Vec<String>,
    pub debug_logs: Vec<String>,
    pub suites: Vec<TestSuiteSummary>,
}

#[derive(Debug, Default, Clone)]
pub struct TestRunRecorder {
    began: bool,
    finished: bool,
    logs: Vec<String>,
    debug_logs: Vec<String>,
    suites: Vec<TestSuiteSummary>,
}

impl TestRunRecorder {
    pub fn apply(&mut self, event: TestExecutionEvent) {
        match event {
            TestExecutionEvent::BeganPlan => self.began = true,
            TestExecutionEvent::FinishedPlan => self.finished = true,
            TestExecutionEvent::Log { message, debug } => {
                if debug {
                    self.debug_logs.push(message);
                } else {
                    self.logs.push(message);
                }
            }
            TestExecutionEvent::SuiteStarted { name, started_at } => {
                self.suites.push(TestSuiteSummary {
                    name,
                    started_at,
                    finished_at: None,
                    test_count: None,
                    skipped: None,
                    failures: None,
                    expected_failures: None,
                    unexpected_failures: None,
                    uncaught_exceptions: None,
                    test_duration_seconds: None,
                    total_duration_seconds: None,
                    cases: Vec::new(),
                });
            }
            TestExecutionEvent::SuiteFinished {
                name,
                finished_at,
                test_count,
                skipped,
                failures,
                expected_failures,
                unexpected_failures,
                uncaught_exceptions,
                test_duration_seconds,
                total_duration_seconds,
            } => {
                let suite = self.find_or_create_suite(&name);
                suite.finished_at = finished_at;
                suite.test_count = Some(test_count);
                suite.skipped = Some(skipped);
                suite.failures = Some(failures);
                suite.expected_failures = Some(expected_failures);
                suite.unexpected_failures = Some(unexpected_failures);
                suite.uncaught_exceptions = Some(uncaught_exceptions);
                suite.test_duration_seconds = Some(test_duration_seconds);
                suite.total_duration_seconds = Some(total_duration_seconds);
            }
            TestExecutionEvent::CaseStarted {
                class_name,
                method_name,
            } => {
                let suite = self.find_or_create_suite(&class_name);
                suite.cases.push(TestCaseSummary {
                    class_name,
                    method_name,
                    status: None,
                    duration_seconds: None,
                    failure: None,
                });
            }
            TestExecutionEvent::CaseFailed {
                class_name,
                method_name,
                message,
                file,
                line,
            } => {
                let case = self.find_or_create_case(&class_name, &method_name);
                case.status = Some(TestCaseStatus::Failed);
                case.failure = Some(TestFailure {
                    message,
                    file,
                    line,
                });
            }
            TestExecutionEvent::CaseFinished {
                class_name,
                method_name,
                status,
                duration_seconds,
            } => {
                let case = self.find_or_create_case(&class_name, &method_name);
                if case.status != Some(TestCaseStatus::Stalled) {
                    case.status = Some(status);
                }
                case.duration_seconds = Some(duration_seconds);
            }
        }
    }

    pub fn summary(&self) -> TestRunSummary {
        let total_tests = self
            .suites
            .iter()
            .map(|suite| suite.test_count.unwrap_or(suite.cases.len() as u64))
            .sum();
        let failed_tests = self
            .suites
            .iter()
            .map(|suite| {
                suite.failures.unwrap_or_else(|| {
                    suite
                        .cases
                        .iter()
                        .filter(|case| case.status == Some(TestCaseStatus::Failed))
                        .count() as u64
                })
            })
            .sum();
        let skipped_tests = self
            .suites
            .iter()
            .map(|suite| {
                suite.skipped.unwrap_or_else(|| {
                    suite
                        .cases
                        .iter()
                        .filter(|case| case.status == Some(TestCaseStatus::Skipped))
                        .count() as u64
                })
            })
            .sum();

        TestRunSummary {
            began: self.began,
            finished: self.finished,
            total_tests,
            failed_tests,
            skipped_tests,
            logs: self.logs.clone(),
            debug_logs: self.debug_logs.clone(),
            suites: self.suites.clone(),
        }
    }

    fn find_or_create_case(&mut self, class_name: &str, method_name: &str) -> &mut TestCaseSummary {
        let suite = self.find_or_create_suite(class_name);
        if let Some(index) = suite
            .cases
            .iter()
            .rposition(|case| case.class_name == class_name && case.method_name == method_name)
        {
            return &mut suite.cases[index];
        }
        suite.cases.push(TestCaseSummary {
            class_name: class_name.to_string(),
            method_name: method_name.to_string(),
            status: None,
            duration_seconds: None,
            failure: None,
        });
        suite.cases.last_mut().expect("just pushed test case")
    }

    fn find_or_create_suite(&mut self, name: &str) -> &mut TestSuiteSummary {
        if let Some(index) = self.suites.iter().rposition(|suite| suite.name == name) {
            return &mut self.suites[index];
        }
        self.suites.push(TestSuiteSummary {
            name: name.to_string(),
            started_at: None,
            finished_at: None,
            test_count: None,
            skipped: None,
            failures: None,
            expected_failures: None,
            unexpected_failures: None,
            uncaught_exceptions: None,
            test_duration_seconds: None,
            total_duration_seconds: None,
            cases: Vec::new(),
        });
        self.suites.last_mut().expect("just pushed suite")
    }
}

fn string_arg(args: &[NSObject], index: usize) -> Option<String> {
    args.get(index)
        .and_then(NSObject::as_str)
        .map(ToString::to_string)
}

fn optional_string_arg(args: &[NSObject], index: usize) -> Option<String> {
    string_arg(args, index).filter(|value| !value.is_empty())
}

fn uint_arg(args: &[NSObject], index: usize) -> Option<u64> {
    match args.get(index)? {
        NSObject::Uint(value) => Some(*value),
        NSObject::Int(value) if *value >= 0 => Some(*value as u64),
        _ => None,
    }
}

fn double_arg(args: &[NSObject], index: usize) -> Option<f64> {
    match args.get(index)? {
        NSObject::Double(value) => Some(*value),
        NSObject::Int(value) => Some(*value as f64),
        NSObject::Uint(value) => Some(*value as f64),
        _ => None,
    }
}

fn identifier_suite_name(value: Option<&NSObject>) -> Option<String> {
    match value? {
        NSObject::String(value) => Some(value.clone()),
        NSObject::Array(values) => values.first().and_then(|value| match value {
            NSObject::String(name) => Some(name.clone()),
            _ => None,
        }),
        NSObject::Dict(dict) => dict
            .get("container")
            .or_else(|| dict.get("suite"))
            .or_else(|| dict.get("testClass"))
            .and_then(NSObject::as_str)
            .map(ToString::to_string),
        _ => None,
    }
}
