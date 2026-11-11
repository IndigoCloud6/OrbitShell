#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Forward arguments directly; never pass authentication URLs through a shell or a log.
    let Some(directory) = std::env::current_exe().ok().and_then(|path| path.parent().map(|path| path.to_path_buf())) else { return; };
    let executable = directory.join(if cfg!(windows) { "OrbitShell.exe" } else { "OrbitShell" });
    let mut command = std::process::Command::new(executable);
    command.args(std::env::args_os().skip(1));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let _ = command.spawn();
}
