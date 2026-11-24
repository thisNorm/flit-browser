//! Startup preferences for the sandboxed Chromium engine.

use cef::{CefString, CommandLine, ImplCommandLine};

pub(crate) fn configure(command: &CommandLine) {
    // This native shell does not display the embedded Chrome component pages.
    command.append_switch(Some(&CefString::from(
        "disable-component-extensions-with-background-pages",
    )));

    command.append_switch_with_value(
        Some(&CefString::from("js-flags")),
        Some(&CefString::from("--optimize-for-size")),
    );

    // Reuse a compatible Chromium renderer for up to four main frames of the
    // same site. Chromium still checks site isolation, storage and frame cost.
    for (switch, feature) in [
        ("disable-features", "ProcessPerSiteForDSE"),
        (
            "enable-features",
            "ProcessPerSiteUpToMainFrameThreshold:ProcessPerSiteMainFrameThreshold/4",
        ),
    ] {
        let switch = CefString::from(switch);
        let existing = CefString::from(&command.switch_value(Some(&switch))).to_string();
        let value = if existing.is_empty() {
            feature.to_owned()
        } else {
            format!("{existing},{feature}")
        };
        command.append_switch_with_value(Some(&switch), Some(&CefString::from(value.as_str())));
    }
    #[cfg(feature = "vault-fixture")]
    crate::vault_login::configure_fixture_tls(command);
}
