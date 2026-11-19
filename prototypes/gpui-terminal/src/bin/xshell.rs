#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    let Ok(adapter) = std::env::current_exe() else {
        return;
    };
    let suffix = std::env::consts::EXE_SUFFIX;
    let installed = adapter.with_file_name(format!("OrbitShell{suffix}"));
    let executable = if installed.is_file() {
        installed
    } else {
        adapter.with_file_name(format!("orbitshell-gpui-terminal{suffix}"))
    };
    let mut command = std::process::Command::new(executable);
    command.args(std::env::args_os().skip(1));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let _ = command.spawn();
}
