//! Install / update / remove / refresh jobs as a step-wise state machine.
//!
//! One call to [`Job::step`] performs the work of exactly one stage, so the
//! window can draw the stage name first and run the (possibly blocking)
//! step on the next frame. The side effects sit behind [`Backend`], which
//! host tests replace with a fake.

use alloc::string::String;

/// What the job does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Install,
    Update,
    Remove,
    /// `pkg update`: fetch and verify the catalogue index.
    Refresh,
}

/// Where the job is. `Done` and `Failed` are final.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    Downloading,
    Verifying,
    Installing,
    Removing,
    /// Refresh: fetching `index.json` and its signature.
    Fetching,
    /// Refresh: checking the signature and parsing the index.
    Checking,
    Done,
    Failed(Failure),
}

/// Why a job stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The server could not be reached.
    Network,
    Http(u16),
    /// The download does not match the checksum in the index.
    Checksum,
    /// The index signature is missing or wrong.
    Signature,
    NotFound,
    Storage,
    Conflict,
    MissingDependency(String),
    /// The index or package could not be read.
    Invalid,
}

impl Failure {
    /// Text shown to the user.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Failure::Network => "No connection. Check the network and try again.".into(),
            Failure::Http(code) => alloc::format!("The server answered with error {code}."),
            Failure::Checksum => "The download is damaged (checksum mismatch).".into(),
            Failure::Signature => {
                "The catalogue signature is not valid. Nothing was changed.".into()
            }
            Failure::NotFound => "The catalogue does not list this app any more.".into(),
            Failure::Storage => "Not enough disk space or a write error.".into(),
            Failure::Conflict => "It conflicts with an installed app.".into(),
            Failure::MissingDependency(dep) => alloc::format!("It needs {dep} installed first."),
            Failure::Invalid => "The package or catalogue could not be read.".into(),
        }
    }

    /// True when the failure means the machine is offline.
    #[must_use]
    pub fn is_offline(&self) -> bool {
        matches!(self, Failure::Network)
    }
}

/// Side effects of a job. Each call corresponds to one stage.
pub trait Backend {
    /// Fetch `index.json` and `index.json.sig`.
    fn fetch_index(&mut self) -> Result<(), Failure>;
    /// Verify the signature of the fetched index and adopt it.
    fn check_index(&mut self) -> Result<(), Failure>;
    fn download(&mut self, name: &str) -> Result<(), Failure>;
    /// Check the downloaded blob against the catalogue checksum.
    fn verify(&mut self, name: &str) -> Result<(), Failure>;
    fn install(&mut self, name: &str) -> Result<(), Failure>;
    fn remove(&mut self, name: &str) -> Result<(), Failure>;
}

/// A running or finished job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub op: Op,
    /// App the job works on; empty for a refresh.
    pub name: String,
    pub stage: Stage,
}

impl Job {
    /// A job at its first stage.
    #[must_use]
    pub fn new(op: Op, name: &str) -> Self {
        let stage = match op {
            Op::Install | Op::Update => Stage::Downloading,
            Op::Remove => Stage::Removing,
            Op::Refresh => Stage::Fetching,
        };
        Self {
            op,
            name: name.into(),
            stage,
        }
    }

    /// True until the job is `Done` or `Failed`.
    #[must_use]
    pub fn is_active(&self) -> bool {
        !matches!(self.stage, Stage::Done | Stage::Failed(_))
    }

    /// Run the current stage and move to the next one.
    pub fn step(&mut self, backend: &mut dyn Backend) {
        let name = self.name.clone();
        let (result, next) = match self.stage {
            Stage::Downloading => (backend.download(&name), Stage::Verifying),
            Stage::Verifying => (backend.verify(&name), Stage::Installing),
            Stage::Installing => (backend.install(&name), Stage::Done),
            Stage::Removing => (backend.remove(&name), Stage::Done),
            Stage::Fetching => (backend.fetch_index(), Stage::Checking),
            Stage::Checking => (backend.check_index(), Stage::Done),
            Stage::Done | Stage::Failed(_) => return,
        };
        self.stage = match result {
            Ok(()) => next,
            Err(failure) => Stage::Failed(failure),
        };
    }

    /// Stage name for the progress bar.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self.stage {
            Stage::Downloading => "Downloading",
            Stage::Verifying => "Verifying",
            Stage::Installing => "Installing",
            Stage::Removing => "Removing",
            Stage::Fetching => "Fetching catalogue",
            Stage::Checking => "Checking signature",
            Stage::Done => "Done",
            Stage::Failed(_) => "Failed",
        }
    }

    /// Finished stages out of all stages, for the progress bar.
    #[must_use]
    pub fn progress(&self) -> (u32, u32) {
        let total = match self.op {
            Op::Install | Op::Update => 3,
            Op::Remove => 1,
            Op::Refresh => 2,
        };
        let done = match (&self.stage, self.op) {
            (Stage::Done, _) => total,
            (Stage::Verifying | Stage::Checking, _) => 1,
            (Stage::Installing, _) => 2,
            _ => 0,
        };
        (done, total)
    }

    /// True when the job failed because the machine is offline.
    #[must_use]
    pub fn stage_is_offline(&self) -> bool {
        matches!(&self.stage, Stage::Failed(f) if f.is_offline())
    }

    /// The error text when the job failed.
    #[must_use]
    pub fn error(&self) -> Option<String> {
        match &self.stage {
            Stage::Failed(failure) => Some(failure.message()),
            _ => None,
        }
    }

    /// One-line summary once the job is over.
    #[must_use]
    pub fn outcome(&self) -> Option<String> {
        let verb = match self.op {
            Op::Install => "Installed",
            Op::Update => "Updated",
            Op::Remove => "Removed",
            Op::Refresh => {
                return matches!(self.stage, Stage::Done).then(|| "Catalogue refreshed.".into())
            }
        };
        match &self.stage {
            Stage::Done => Some(alloc::format!("{verb} {}.", self.name)),
            Stage::Failed(failure) => Some(alloc::format!(
                "{} failed: {}",
                self.name,
                failure.message()
            )),
            _ => None,
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// Fails at the named call; records every call made.
    #[derive(Default)]
    struct Fake {
        fail_at: Option<(&'static str, Failure)>,
        calls: Vec<&'static str>,
    }

    impl Fake {
        fn run(&mut self, call: &'static str) -> Result<(), Failure> {
            self.calls.push(call);
            match &self.fail_at {
                Some((at, failure)) if *at == call => Err(failure.clone()),
                _ => Ok(()),
            }
        }
    }

    impl Backend for Fake {
        fn fetch_index(&mut self) -> Result<(), Failure> {
            self.run("fetch")
        }
        fn check_index(&mut self) -> Result<(), Failure> {
            self.run("check")
        }
        fn download(&mut self, _: &str) -> Result<(), Failure> {
            self.run("download")
        }
        fn verify(&mut self, _: &str) -> Result<(), Failure> {
            self.run("verify")
        }
        fn install(&mut self, _: &str) -> Result<(), Failure> {
            self.run("install")
        }
        fn remove(&mut self, _: &str) -> Result<(), Failure> {
            self.run("remove")
        }
    }

    fn run_to_end(job: &mut Job, fake: &mut Fake) -> usize {
        let mut steps = 0;
        while job.is_active() {
            job.step(fake);
            steps += 1;
        }
        steps
    }

    #[test]
    fn install_walks_download_verify_install_done() {
        let mut job = Job::new(Op::Install, "mines");
        let mut fake = Fake::default();
        assert_eq!(job.stage, Stage::Downloading);
        job.step(&mut fake);
        assert_eq!(job.stage, Stage::Verifying);
        job.step(&mut fake);
        assert_eq!(job.stage, Stage::Installing);
        job.step(&mut fake);
        assert_eq!(job.stage, Stage::Done);
        assert_eq!(fake.calls, ["download", "verify", "install"]);
        assert_eq!(job.outcome().as_deref(), Some("Installed mines."));
        assert_eq!(job.progress(), (3, 3));
    }

    #[test]
    fn checksum_failure_stops_before_installing() {
        let mut job = Job::new(Op::Update, "mines");
        let mut fake = Fake {
            fail_at: Some(("verify", Failure::Checksum)),
            ..Fake::default()
        };
        run_to_end(&mut job, &mut fake);
        assert_eq!(job.stage, Stage::Failed(Failure::Checksum));
        assert_eq!(fake.calls, ["download", "verify"]);
        assert!(job.error().is_some_and(|e| e.contains("checksum")));
    }

    #[test]
    fn signature_failure_rejects_the_refresh() {
        let mut job = Job::new(Op::Refresh, "");
        let mut fake = Fake {
            fail_at: Some(("check", Failure::Signature)),
            ..Fake::default()
        };
        run_to_end(&mut job, &mut fake);
        assert_eq!(job.stage, Stage::Failed(Failure::Signature));
        assert!(job.error().is_some_and(|e| e.contains("signature")));
        assert!(!job.stage_is_offline());
    }

    #[test]
    fn network_failure_reports_offline() {
        let mut job = Job::new(Op::Install, "mines");
        let mut fake = Fake {
            fail_at: Some(("download", Failure::Network)),
            ..Fake::default()
        };
        let steps = run_to_end(&mut job, &mut fake);
        assert_eq!(steps, 1);
        assert!(job.stage_is_offline());
        assert_eq!(fake.calls, ["download"]);
    }

    #[test]
    fn remove_and_refresh_have_their_own_stages() {
        let mut fake = Fake::default();
        let mut remove = Job::new(Op::Remove, "mines");
        assert_eq!(run_to_end(&mut remove, &mut fake), 1);
        assert_eq!(remove.outcome().as_deref(), Some("Removed mines."));
        let mut refresh = Job::new(Op::Refresh, "");
        assert_eq!(run_to_end(&mut refresh, &mut fake), 2);
        assert_eq!(fake.calls, ["remove", "fetch", "check"]);
        assert_eq!(refresh.outcome().as_deref(), Some("Catalogue refreshed."));
    }

    #[test]
    fn finished_job_ignores_further_steps() {
        let mut job = Job::new(Op::Remove, "x");
        let mut fake = Fake::default();
        run_to_end(&mut job, &mut fake);
        job.step(&mut fake);
        assert_eq!(fake.calls.len(), 1);
    }
}
