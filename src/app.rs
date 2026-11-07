//! Native Shell lifecycle for the desktop Cargo launcher.
//!
//! `nsh run` and `nsh package` generate the corresponding NSH runner and web
//! runtime from the same `crate::App` root. This module only exists so
//! `cargo run` remains a useful local desktop smoke check.

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
pub fn run() -> impl std::process::Termination {
    #[cfg(target_os = "linux")]
    use nsh_platform_linux as platform;
    #[cfg(target_os = "macos")]
    use nsh_platform_macos as platform;
    #[cfg(target_os = "windows")]
    use nsh_platform_windows as platform;

    env_logger::init();
    platform::set_ui_sets(vec![nsh_base::registry(), crate::registry()]);
    platform::run_app(
        platform::AppSource::Rust(
            platform::Root::of::<crate::App>()
                .app_id("com.latticetrading.lattice")
                .tokens(crate::theme::tokens())
                .font(
                    include_bytes!("../assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf")
                        .to_vec(),
                )
                .font(
                    include_bytes!("../assets/fonts/ibm-plex-mono/IBMPlexMono-Medium.ttf").to_vec(),
                )
                .font(
                    include_bytes!("../assets/fonts/ibm-plex-mono/IBMPlexMono-SemiBold.ttf")
                        .to_vec(),
                )
                .font(include_bytes!("../assets/fonts/ibm-plex-mono/IBMPlexMono-Bold.ttf").to_vec())
                .appearance(nsh_core::platform::Appearance::Dark),
        ),
        platform::AppOptions {
            title: "Lattice".to_owned(),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            app_id: "com.latticetrading.lattice".to_owned(),
            size: (1280, 800),
            appearance: platform::Appearance::Dark,
            ..Default::default()
        },
    )
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
pub fn run() -> impl std::process::Termination {
    std::process::ExitCode::SUCCESS
}
