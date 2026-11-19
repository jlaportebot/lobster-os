//! Network: link status, hostname, connection test.

use super::{heading, info_card, note, setting, Flow};
use crate::apps::settings::model::Action;
use crate::apps::settings::state::{NetTest, State, DEFAULT_TEST_URL};
use crate::apps::settings::system;
use crate::wm::controls::{ButtonKind, Ui};
use alloc::string::String;

/// Width of the URL field.
const URL_W: u32 = 300;
/// Width of the test button.
const TEST_W: u32 = 160;

pub fn draw(ui: &mut Ui, flow: &mut Flow, st: &mut State) {
    heading(ui, flow, "CONNECTION");
    let mut lines = alloc::vec![
        ("Status", String::from(system::link_status())),
        ("Hostname", String::from(system::HOSTNAME)),
    ];
    push_address(&mut lines);
    info_card(ui, flow, &lines);
    flow.space(8);
    heading(ui, flow, "TEST CONNECTION");
    let field = setting(
        ui,
        flow,
        "Address to fetch",
        "Fetched like the browser does",
        URL_W,
    );
    ui.text_field(Action::UrlField.key(), field, &st.url, DEFAULT_TEST_URL);
    let can_test =
        system::has_network() && !matches!(st.net_test, NetTest::Queued | NetTest::Running);
    let button = setting(ui, flow, "Test connection", "", TEST_W);
    ui.button(
        Action::TestConnection.key(),
        button,
        "Test connection",
        ButtonKind::Primary,
        can_test,
    );
    note(ui, flow, &result_line(st));
}

fn push_address(lines: &mut alloc::vec::Vec<(&'static str, String)>) {
    #[cfg(target_arch = "aarch64")]
    {
        let ip = crate::net::config().ip_addr;
        if ip != [0; 4] {
            lines.push((
                "IP address",
                alloc::format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]),
            ));
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = lines;
    }
}

fn result_line(st: &State) -> String {
    match &st.net_test {
        NetTest::Idle if system::has_network() => String::new(),
        NetTest::Idle => String::from("No network device: nothing to test."),
        NetTest::Queued | NetTest::Running => String::from("Testing..."),
        NetTest::Done(report) => report.clone(),
    }
}
