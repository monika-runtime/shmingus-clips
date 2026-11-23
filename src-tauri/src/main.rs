// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    shmingus_clips_lib::prepare_runtime_environment();
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--doctor") {
        shmingus_clips_lib::diagnostics::doctor();
        return;
    }
    if args.iter().any(|a| a == "--selftest") {
        shmingus_clips_lib::diagnostics::selftest();
        return;
    }

    shmingus_clips_lib::run()
}
