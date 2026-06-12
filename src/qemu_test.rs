//! QEMU Integration Test Framework
//!
//! Provides infrastructure for running integration tests in QEMU.
//! Tests are registered and executed during kernel boot when `test_mode=1` is passed.

use crate::print;
use crate::println;

/// Test result
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TestResult {
    Passed,
    Failed,
    Skipped,
}

/// Test function type
pub type TestFn = fn() -> TestResult;

/// Test case
pub struct TestCase {
    pub name: &'static str,
    pub func: TestFn,
}

/// Global test registry
static mut TEST_REGISTRY: Option<&'static [TestCase]> = None;

/// Register tests (called once at startup)
pub fn register_tests(tests: &'static [TestCase]) {
    unsafe {
        TEST_REGISTRY = Some(tests);
    }
}

/// Run all registered tests
pub fn run_tests() -> bool {
    let tests = unsafe { TEST_REGISTRY };
    if tests.is_none() {
        println!("[test] No tests registered");
        return true;
    }

    let tests = tests.unwrap();
    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;

    println!("[test] Running {} integration tests...", tests.len());

    for test in tests {
        print!("[test] {} ... ", test.name);
        let result = (test.func)();
        match result {
            TestResult::Passed => {
                println!("PASSED");
                passed += 1;
            }
            TestResult::Failed => {
                println!("FAILED");
                failed += 1;
            }
            TestResult::Skipped => {
                println!("SKIPPED");
                skipped += 1;
            }
        }
    }

    println!(
        "[test] Results: {} passed, {} failed, {} skipped",
        passed, failed, skipped
    );

    if failed == 0 {
        println!("ALL TESTS PASSED");
        true
    } else {
        println!("TEST FAILED");
        false
    }
}

/// Assertion macros for tests
#[macro_export]
macro_rules! test_assert {
    ($cond:expr) => {
        if !$cond {
            return $crate::qemu_test::TestResult::Failed;
        }
    };
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            println!($($arg)*);
            return $crate::qemu_test::TestResult::Failed;
        }
    };
}

#[macro_export]
macro_rules! test_assert_eq {
    ($left:expr, $right:expr) => {
        if $left != $right {
            println!("[test] assert_eq failed: left={:?}, right={:?}", $left, $right);
            return $crate::qemu_test::TestResult::Failed;
        }
    };
    ($left:expr, $right:expr, $($arg:tt)*) => {
        if $left != $right {
            println!($($arg)*);
            println!("[test] assert_eq failed: left={:?}, right={:?}", $left, $right);
            return $crate::qemu_test::TestResult::Failed;
        }
    };
}

#[macro_export]
macro_rules! test_assert_ne {
    ($left:expr, $right:expr) => {
        if $left == $right {
            println!("[test] assert_ne failed: both are {:?}", $left);
            return $crate::qemu_test::TestResult::Failed;
        }
    };
}

/// Helper to create test cases
#[macro_export]
macro_rules! test_case {
    ($name:expr, $func:expr) => {
        $crate::qemu_test::TestCase {
            name: $name,
            func: $func,
        }
    };
}

/// Test suite macro
#[macro_export]
macro_rules! test_suite {
    ($($name:expr => $func:expr),*) => {
        &[
            $(test_case!($name, $func)),*
        ]
    };
}
