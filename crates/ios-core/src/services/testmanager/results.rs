//! XCTest result event parsing and run summary accumulation.
//!
//! Testmanager reports progress as DTX method invocations using private XCTest
//! selectors. This module translates the selectors into stable Rust events and
//! accumulates those events into a serializable summary for CLI and binding users.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::services::dtx::{DtxMessage, DtxPayload, NSObject};
use serde::Serialize;

const MAX_CAPTURED_LOG_MESSAGES: usize = 16_384;
const MAX_CAPTURED_LOG_BYTES: usize = 8 * 1024 * 1024;
const MAX_JUNIT_DIAGNOSTIC_BYTES: usize = 64 * 1024;

/// XCTest selector emitted when a test plan begins.
pub const DID_BEGIN_EXECUTING_TEST_PLAN_SELECTOR: &str = "_XCT_didBeginExecutingTestPlan";
/// XCTest selector emitted when a test plan finishes.
pub const DID_FINISH_EXECUTING_TEST_PLAN_SELECTOR: &str = "_XCT_didFinishExecutingTestPlan";
/// XCTest selector for normal log messages.
pub const LOG_MESSAGE_SELECTOR: &str = "_XCT_logMessage:";
/// XCTest selector for debug log messages.
pub const LOG_DEBUG_MESSAGE_SELECTOR: &str = "_XCT_logDebugMessage:";
/// XCTest selector emitted when a suite starts.
pub const TEST_SUITE_STARTED_SELECTOR: &str = "_XCT_testSuite:didStartAt:";
/// XCTest selector emitted when older XCTest runtimes finish a suite.
pub const TEST_SUITE_FINISHED_SELECTOR: &str =
    "_XCT_testSuite:didFinishAt:runCount:withFailures:unexpected:testDuration:totalDuration:";
/// XCTest selector emitted when newer XCTest runtimes finish a suite with skip counts.
pub const TEST_SUITE_FINISHED_WITH_SKIP_SELECTOR: &str =
    "_XCT_testSuiteWithIdentifier:didFinishAt:runCount:skipCount:failureCount:expectedFailureCount:uncaughtExceptionCount:testDuration:totalDuration:";
/// XCTest selector emitted when a test case starts.
pub const TEST_CASE_STARTED_SELECTOR: &str = "_XCT_testCaseDidStartForTestClass:method:";
/// XCTest selector emitted when a test case finishes.
pub const TEST_CASE_FINISHED_SELECTOR: &str =
    "_XCT_testCaseDidFinishForTestClass:method:withStatus:duration:";
/// XCTest selector emitted when a test case records a failure.
pub const TEST_CASE_FAILED_SELECTOR: &str =
    "_XCT_testCaseDidFailForTestClass:method:withMessage:file:line:";

/// A normalized XCTest execution event decoded from a DTX method invocation.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TestExecutionEvent {
    /// The test plan started.
    BeganPlan,
    /// The test plan finished.
    FinishedPlan,
    /// XCTest emitted a log message.
    Log {
        /// Log message text.
        message: String,
        /// Whether the message came from the debug log selector.
        debug: bool,
    },
    /// A test suite started.
    SuiteStarted {
        /// Suite name.
        name: String,
        /// Start timestamp string as reported by XCTest.
        started_at: Option<String>,
    },
    /// A test suite finished and reported aggregate counts.
    SuiteFinished {
        /// Suite name.
        name: String,
        /// Finish timestamp string as reported by XCTest.
        finished_at: Option<String>,
        /// Number of tests reported by the suite.
        test_count: u64,
        /// Number of skipped tests.
        skipped: u64,
        /// Number of failures.
        failures: u64,
        /// Number of expected failures.
        expected_failures: u64,
        /// Number of unexpected failures.
        unexpected_failures: u64,
        /// Number of uncaught exceptions.
        uncaught_exceptions: u64,
        /// XCTest execution duration in seconds.
        test_duration_seconds: f64,
        /// Total suite duration in seconds.
        total_duration_seconds: f64,
    },
    /// A test case started.
    CaseStarted {
        /// XCTest class name.
        class_name: String,
        /// XCTest method name.
        method_name: String,
    },
    /// A test case reported a failure.
    CaseFailed {
        /// XCTest class name.
        class_name: String,
        /// XCTest method name.
        method_name: String,
        /// Failure message.
        message: String,
        /// Source file when XCTest reports one.
        file: Option<String>,
        /// Source line when XCTest reports one.
        line: Option<u64>,
    },
    /// A test case finished with a final status.
    CaseFinished {
        /// XCTest class name.
        class_name: String,
        /// XCTest method name.
        method_name: String,
        /// Final test status.
        status: TestCaseStatus,
        /// Test case duration in seconds.
        duration_seconds: f64,
    },
}

impl TestExecutionEvent {
    /// Decode a supported XCTest DTX method invocation.
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

    /// Return true when this event marks the end of the plan.
    pub fn is_finished_plan(&self) -> bool {
        matches!(self, Self::FinishedPlan)
    }
}

/// Normalized XCTest case status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TestCaseStatus {
    /// Test passed.
    Passed,
    /// Test failed.
    Failed,
    /// XCTest reported an expected failure.
    ExpectedFailure,
    /// XCTest reported a stalled case.
    Stalled,
    /// Test was skipped.
    Skipped,
    /// Status string not modeled by ios-core yet.
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

/// Failure details for a single test case.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestFailure {
    /// Failure message emitted by XCTest.
    pub message: String,
    /// Source file path when available.
    pub file: Option<String>,
    /// Source line when available.
    pub line: Option<u64>,
}

/// Summary for one XCTest case.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestCaseSummary {
    /// XCTest class name.
    pub class_name: String,
    /// XCTest method name.
    pub method_name: String,
    /// Final case status, if observed.
    pub status: Option<TestCaseStatus>,
    /// Case duration in seconds, if reported.
    pub duration_seconds: Option<f64>,
    /// First failure associated with the case, if any.
    pub failure: Option<TestFailure>,
}

/// Summary for one XCTest suite.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestSuiteSummary {
    /// Suite name.
    pub name: String,
    /// Start timestamp string as reported by XCTest.
    pub started_at: Option<String>,
    /// Finish timestamp string as reported by XCTest.
    pub finished_at: Option<String>,
    /// Total tests reported by the suite.
    pub test_count: Option<u64>,
    /// Skipped test count.
    pub skipped: Option<u64>,
    /// Failure count.
    pub failures: Option<u64>,
    /// Expected failure count.
    pub expected_failures: Option<u64>,
    /// Unexpected failure count.
    pub unexpected_failures: Option<u64>,
    /// Uncaught exception count.
    pub uncaught_exceptions: Option<u64>,
    /// XCTest execution duration in seconds.
    pub test_duration_seconds: Option<f64>,
    /// Total suite duration in seconds.
    pub total_duration_seconds: Option<f64>,
    /// Case summaries accumulated for this suite.
    pub cases: Vec<TestCaseSummary>,
}

/// Summary for an XCTest run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TestRunSummary {
    /// Whether a plan-start event was observed.
    pub began: bool,
    /// Whether a plan-finish event was observed.
    pub finished: bool,
    /// Total test count across suites.
    pub total_tests: u64,
    /// Total failed test count across suites.
    pub failed_tests: u64,
    /// Total skipped test count across suites.
    pub skipped_tests: u64,
    /// Non-debug log messages.
    pub logs: Vec<String>,
    /// Debug log messages.
    pub debug_logs: Vec<String>,
    /// Suite summaries.
    pub suites: Vec<TestSuiteSummary>,
}

/// Stateful accumulator for XCTest events.
#[derive(Debug, Default, Clone)]
pub struct TestRunRecorder {
    began: bool,
    finished: bool,
    logs: Vec<String>,
    debug_logs: Vec<String>,
    log_bytes: usize,
    debug_log_bytes: usize,
    suites: Vec<TestSuiteSummary>,
}

impl TestRunRecorder {
    /// Apply one event to the current run summary.
    pub fn apply(&mut self, event: TestExecutionEvent) {
        match event {
            TestExecutionEvent::BeganPlan => self.began = true,
            TestExecutionEvent::FinishedPlan => self.finished = true,
            TestExecutionEvent::Log { message, debug } => {
                self.push_log(message, debug);
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
                // XCTest can report a suite name different from its test class. The Go listener
                // attaches such cases to the currently running suite rather than creating a
                // second class-named suite.
                let suite = self.find_or_create_active_suite(&class_name);
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

    /// Build a serializable summary from the events applied so far.
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
                        .filter(|case| {
                            matches!(
                                case.status,
                                Some(TestCaseStatus::Skipped)
                                    | Some(TestCaseStatus::ExpectedFailure)
                            )
                        })
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

    fn push_log(&mut self, mut message: String, debug: bool) {
        let (logs, used) = if debug {
            (&mut self.debug_logs, &mut self.debug_log_bytes)
        } else {
            (&mut self.logs, &mut self.log_bytes)
        };
        if logs.len() >= MAX_CAPTURED_LOG_MESSAGES || *used >= MAX_CAPTURED_LOG_BYTES {
            return;
        }
        let remaining = MAX_CAPTURED_LOG_BYTES - *used;
        if message.len() > remaining {
            let mut end = remaining;
            while end > 0 && !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
        }
        if message.is_empty() {
            return;
        }
        *used = (*used).saturating_add(message.len());
        logs.push(message);
    }

    fn find_or_create_case(&mut self, class_name: &str, method_name: &str) -> &mut TestCaseSummary {
        let suite = self.find_or_create_active_suite(class_name);
        if let Some(index) = suite
            .cases
            .iter()
            .rposition(|case| case.class_name == class_name && case.method_name == method_name)
        {
            return &mut suite.cases[index];
        }
        let index = suite.cases.len();
        suite.cases.push(TestCaseSummary {
            class_name: class_name.to_string(),
            method_name: method_name.to_string(),
            status: None,
            duration_seconds: None,
            failure: None,
        });
        &mut suite.cases[index]
    }

    fn find_or_create_suite(&mut self, name: &str) -> &mut TestSuiteSummary {
        if let Some(index) = self.suites.iter().rposition(|suite| suite.name == name) {
            return &mut self.suites[index];
        }
        let index = self.suites.len();
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
        &mut self.suites[index]
    }

    fn find_or_create_active_suite(&mut self, class_name: &str) -> &mut TestSuiteSummary {
        if let Some(index) = self
            .suites
            .iter()
            .rposition(|suite| suite.test_count.is_none())
        {
            return &mut self.suites[index];
        }
        self.find_or_create_suite(class_name)
    }
}

impl TestRunSummary {
    /// Serialize this result summary as JUnit XML.
    ///
    /// Expected failures retain an explicit status property and use JUnit's standard `<skipped>`
    /// element, matching go-ios. Stalled/unknown cases use `<error>` so consumers do not mistake
    /// an incomplete result for a passing test.
    pub fn to_junit_xml(&self) -> String {
        self.to_junit_xml_with_diagnostic(None)
    }

    /// Serialize this result summary as JUnit XML and include a diagnostic
    /// error when startup/result collection was incomplete.
    pub fn to_junit_xml_with_diagnostic(&self, diagnostic: Option<&str>) -> String {
        let mut suites = String::new();
        let mut total_tests = 0u64;
        let mut total_failures = 0u64;
        let mut total_errors = 0u64;
        let mut total_skipped = 0u64;
        let mut total_time = 0.0f64;

        for suite in &self.suites {
            let report = JUnitSuiteReport::from_suite(suite);
            total_tests = total_tests.saturating_add(report.tests);
            total_failures = total_failures.saturating_add(report.failures);
            total_errors = total_errors.saturating_add(report.errors);
            total_skipped = total_skipped.saturating_add(report.skipped);
            total_time += report.time;
            render_junit_suite(&mut suites, suite, &report);
        }

        let diagnostic = diagnostic
            .map(|value| truncate_utf8(value, MAX_JUNIT_DIAGNOSTIC_BYTES))
            .or_else(|| (!self.finished).then(|| "XCTest run did not finish".to_string()));
        if let Some(diagnostic) = diagnostic.as_deref() {
            total_errors = total_errors.saturating_add(1);
            suites.push_str("<testsuite");
            xml_attr(&mut suites, "name", "xctest-diagnostic");
            xml_attr(&mut suites, "tests", "0");
            xml_attr(&mut suites, "failures", "0");
            xml_attr(&mut suites, "errors", "1");
            xml_attr(&mut suites, "skipped", "0");
            xml_attr(&mut suites, "time", "0.000");
            suites.push('>');
            suites.push_str("<error");
            xml_attr(&mut suites, "type", "xctest_diagnostic");
            xml_attr(&mut suites, "message", diagnostic);
            suites.push('>');
            xml_text(&mut suites, diagnostic);
            suites.push_str("</error></testsuite>");
        }

        if !self.logs.is_empty() || !self.debug_logs.is_empty() {
            suites.push_str("<testsuite");
            xml_attr(&mut suites, "name", "xctest-output");
            xml_attr(&mut suites, "tests", "0");
            xml_attr(&mut suites, "failures", "0");
            xml_attr(&mut suites, "errors", "0");
            xml_attr(&mut suites, "skipped", "0");
            xml_attr(&mut suites, "time", "0.000");
            suites.push('>');
            if !self.logs.is_empty() {
                suites.push_str("<system-out>");
                xml_text(&mut suites, &self.logs.join("\n"));
                suites.push_str("</system-out>");
            }
            if !self.debug_logs.is_empty() {
                suites.push_str("<system-err>");
                xml_text(&mut suites, &self.debug_logs.join("\n"));
                suites.push_str("</system-err>");
            }
            suites.push_str("</testsuite>");
        }

        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"XCTest\" tests=\"{total_tests}\" failures=\"{total_failures}\" errors=\"{total_errors}\" skipped=\"{total_skipped}\" time=\"{:.3}\">{suites}</testsuites>\n",
            finite_duration(total_time)
        )
    }
}

/// Atomically write a JUnit report next to the requested destination.
///
/// The temporary file is created in the destination directory, flushed, and
/// renamed into place. On Unix, rename atomically replaces an existing file; Windows uses
/// MoveFileEx(REPLACE_EXISTING) so overwriting an existing report has the same contract.
pub fn write_junit_xml_atomic(summary: &TestRunSummary, path: &Path) -> io::Result<()> {
    write_junit_xml_text_atomic(&summary.to_junit_xml(), path)
}

/// Atomically write a JUnit report with a startup/result-collection diagnostic.
pub fn write_junit_xml_atomic_with_diagnostic(
    summary: &TestRunSummary,
    path: &Path,
    diagnostic: &str,
) -> io::Result<()> {
    write_junit_xml_text_atomic(
        &summary.to_junit_xml_with_diagnostic(Some(diagnostic)),
        path,
    )
}

fn write_junit_xml_text_atomic(xml: &str, path: &Path) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "JUnit output path must name a file",
        )
    })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();

    for attempt in 0..16u32 {
        let temporary = parent.join(format!(
            ".{}.ios-junit-{pid}-{nonce}-{attempt}.tmp",
            file_name.to_string_lossy()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&temporary) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let result = file.write_all(xml.as_bytes()).and_then(|_| file.sync_all());
        drop(file);
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        match replace_file_atomically(&temporary, path) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(error);
            }
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique temporary JUnit output path",
    ))
}

#[cfg(windows)]
fn replace_file_atomically(temporary: &Path, destination: &Path) -> io::Result<()> {
    crate::fs_replace::move_file_replace(temporary, destination)
}

#[cfg(not(windows))]
fn replace_file_atomically(temporary: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(temporary, destination)
}

#[derive(Debug, Clone, Copy)]
struct JUnitSuiteReport {
    tests: u64,
    failures: u64,
    errors: u64,
    skipped: u64,
    time: f64,
}

impl JUnitSuiteReport {
    fn from_suite(suite: &TestSuiteSummary) -> Self {
        let case_failures = suite
            .cases
            .iter()
            .filter(|case| case.status == Some(TestCaseStatus::Failed))
            .count() as u64;
        let case_errors = suite
            .cases
            .iter()
            .filter(|case| {
                matches!(
                    case.status,
                    None | Some(TestCaseStatus::Stalled) | Some(TestCaseStatus::Other(_))
                )
            })
            .count() as u64;
        let case_skipped = suite
            .cases
            .iter()
            .filter(|case| {
                matches!(
                    case.status,
                    Some(TestCaseStatus::Skipped) | Some(TestCaseStatus::ExpectedFailure)
                )
            })
            .count() as u64;
        // JUnit counts concrete testcase children. XCTest's suite counter may include cases for
        // which no event was delivered, but emitting a larger count would violate the common
        // `tests == number of testcase elements` contract used by go-ios consumers.
        let tests = suite.cases.len() as u64;
        // JUnit's suite time is execution time. XCTest's totalDuration also includes setup and
        // teardown, so match go-ios and use testDuration, falling back to case durations when
        // XCTest reported zero for an incomplete suite.
        let test_duration = suite.test_duration_seconds.unwrap_or(0.0);
        let time = if test_duration.is_finite() && test_duration > 0.0 {
            test_duration
        } else {
            suite
                .cases
                .iter()
                .filter_map(|case| case.duration_seconds)
                .sum()
        };
        Self {
            tests,
            failures: case_failures,
            errors: suite
                .uncaught_exceptions
                .unwrap_or(0)
                .saturating_add(case_errors),
            skipped: case_skipped,
            time: finite_duration(time),
        }
    }
}

fn render_junit_suite(output: &mut String, suite: &TestSuiteSummary, report: &JUnitSuiteReport) {
    output.push_str("<testsuite");
    xml_attr(output, "name", &suite.name);
    xml_attr(output, "tests", &report.tests.to_string());
    xml_attr(output, "failures", &report.failures.to_string());
    xml_attr(output, "errors", &report.errors.to_string());
    xml_attr(output, "skipped", &report.skipped.to_string());
    xml_attr(output, "time", &format!("{:.3}", report.time));
    output.push('>');
    if let Some(expected_failures) = suite.expected_failures {
        output.push_str("<properties><property");
        xml_attr(output, "name", "expected_failures");
        xml_attr(output, "value", &expected_failures.to_string());
        output.push_str("/></properties>");
    }

    for case in &suite.cases {
        let duration = finite_duration(case.duration_seconds.unwrap_or(0.0));
        output.push_str("<testcase");
        xml_attr(output, "classname", &case.class_name);
        xml_attr(output, "name", &case.method_name);
        xml_attr(output, "time", &format!("{duration:.3}"));
        output.push('>');
        match case.status.as_ref() {
            Some(TestCaseStatus::Failed) => {
                if let Some(failure) = &case.failure {
                    render_failure(output, failure);
                } else {
                    output.push_str("<failure type=\"XCTestFailure\" message=\"failed\"/>");
                }
            }
            Some(TestCaseStatus::Skipped) => output.push_str("<skipped/>"),
            Some(TestCaseStatus::ExpectedFailure) => {
                output.push_str("<skipped");
                xml_attr(output, "message", "expected failure");
                output.push_str("/>");
                render_status_property(output, "expected_failure");
            }
            Some(TestCaseStatus::Stalled) => {
                render_error(output, "stalled", "XCTest case stalled");
            }
            Some(TestCaseStatus::Other(status)) => {
                render_status_property(output, status);
                render_error(output, "unknown_status", status);
            }
            None => {
                render_status_property(output, "unknown");
                render_error(
                    output,
                    "incomplete",
                    "XCTest case did not report a final status",
                );
            }
            Some(TestCaseStatus::Passed) => {}
        }
        output.push_str("</testcase>");
    }
    output.push_str("</testsuite>");
}

fn render_failure(output: &mut String, failure: &TestFailure) {
    output.push_str("<failure");
    xml_attr(output, "type", "XCTestFailure");
    xml_attr(output, "message", &failure.message);
    output.push('>');
    if let Some(file) = &failure.file {
        xml_text(output, file);
        if let Some(line) = failure.line {
            output.push(':');
            output.push_str(&line.to_string());
        }
        output.push_str(": ");
    }
    xml_text(output, &failure.message);
    output.push_str("</failure>");
}

fn render_error(output: &mut String, error_type: &str, message: &str) {
    output.push_str("<error");
    xml_attr(output, "type", error_type);
    xml_attr(output, "message", message);
    output.push_str("/>");
}

fn render_status_property(output: &mut String, status: &str) {
    output.push_str("<properties><property");
    xml_attr(output, "name", "status");
    xml_attr(output, "value", status);
    output.push_str("/></properties>");
}

fn xml_attr(output: &mut String, name: &str, value: &str) {
    output.push(' ');
    output.push_str(name);
    output.push_str("=\"");
    xml_text(output, value);
    output.push('"');
}

fn xml_text(output: &mut String, value: &str) {
    for character in value.chars() {
        if !is_xml_10_character(character) {
            output.push('\u{FFFD}');
            continue;
        }
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            character => output.push(character),
        }
    }
}

fn is_xml_10_character(character: char) -> bool {
    matches!(
        character,
        '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'
    )
}

fn finite_duration(value: f64) -> f64 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        0.0
    }
}

fn truncate_utf8(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
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
