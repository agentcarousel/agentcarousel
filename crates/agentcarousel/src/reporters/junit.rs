use agentcarousel_core::{CaseStatus, Run};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename = "testsuite")]
struct TestSuite {
    #[serde(rename = "@name")]
    name: String,
    #[serde(rename = "@tests")]
    tests: usize,
    #[serde(rename = "@failures")]
    failures: usize,
    #[serde(rename = "@errors")]
    errors: usize,
    #[serde(rename = "testcase")]
    testcases: Vec<TestCase>,
}

#[derive(Serialize)]
struct TestCase {
    #[serde(rename = "@name")]
    name: String,
    #[serde(rename = "failure", skip_serializing_if = "Option::is_none")]
    failure: Option<Failure>,
    #[serde(rename = "error", skip_serializing_if = "Option::is_none")]
    error: Option<Failure>,
}

#[derive(Serialize)]
struct Failure {
    #[serde(rename = "$text")]
    message: String,
}

pub fn print_junit(run: &Run) {
    let mut failures = 0;
    let mut errors = 0;
    let mut testcases = Vec::new();

    for case in &run.cases {
        let mut failure = None;
        let mut error = None;
        match case.status {
            CaseStatus::Failed => {
                failures += 1;
                failure = Some(Failure {
                    message: case.error.clone().unwrap_or_else(|| "failed".to_string()),
                });
            }
            CaseStatus::Flaky => {
                failures += 1;
                failure = Some(Failure {
                    message: case.error.clone().unwrap_or_else(|| "flaky".to_string()),
                });
            }
            CaseStatus::TimedOut | CaseStatus::Error => {
                errors += 1;
                error = Some(Failure {
                    message: case.error.clone().unwrap_or_else(|| "error".to_string()),
                });
            }
            _ => {}
        }
        testcases.push(TestCase {
            name: case.case_id.0.clone(),
            failure,
            error,
        });
    }

    let suite = TestSuite {
        name: "agentcarousel".to_string(),
        tests: run.cases.len(),
        failures,
        errors,
        testcases,
    };

    let xml = quick_xml::se::to_string(&suite).unwrap_or_else(|_| "<testsuite />".to_string());
    println!("{xml}");
}
