use std::fmt::Write as _;

use zeroize::Zeroizing;

use super::{check_command, lex, Token, Word};

/// A single allowlisted diagnostic, with its executable and argument semantics
/// fixed before it can enter the scoped approval path.
pub struct ReadOnlyCommand(Zeroizing<String>);

impl ReadOnlyCommand {
    pub fn prepare(input: &str) -> Option<Self> {
        let source = input.trim_matches([' ', '\r', '\n']);
        if source.len() > 4096
            || !source.chars().all(|character| {
                character.is_alphanumeric() || " -_./:=@,+%\"'".contains(character)
            })
        {
            return None;
        }
        check_command(input).ok()?;
        let tokens = lex(source).ok()?;
        let args = tokens
            .iter()
            .map(|token| match token {
                Token::Word(Word {
                    text,
                    dynamic: false,
                }) => Some(text.as_str()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        let (&requested, args) = args.split_first()?;
        if args.len() > 32 {
            return None;
        }
        let program = requested
            .strip_prefix("/usr/bin/")
            .or_else(|| requested.strip_prefix("/bin/"))
            .unwrap_or(requested);
        let mut pager = false;
        let mut journal_limit = false;
        let accepted = match program {
            "pwd" => simple_flags(args, "LP", &["--logical", "--physical"], false),
            "ls" => simple_flags(
                args,
                "aAlhtrd1FpSUi",
                &[
                    "--all",
                    "--almost-all",
                    "--human-readable",
                    "--directory",
                    "--reverse",
                ],
                true,
            ),
            "df" => simple_flags(
                args,
                "ahHPTi",
                &[
                    "--all",
                    "--human-readable",
                    "--portability",
                    "--print-type",
                    "--inodes",
                ],
                true,
            ),
            "du" => simple_flags(
                args,
                "ahsx",
                &[
                    "--all",
                    "--human-readable",
                    "--summarize",
                    "--one-file-system",
                ],
                true,
            ),
            "free" => simple_flags(args, "bhkmgwt", &["--human", "--wide", "--total"], false),
            "uname" => simple_flags(
                args,
                "asnrvmpio",
                &[
                    "--all",
                    "--kernel-name",
                    "--nodename",
                    "--kernel-release",
                    "--machine",
                ],
                false,
            ),
            "uptime" => simple_flags(args, "ps", &["--pretty", "--since"], false),
            "id" => simple_flags(
                args,
                "ugGn",
                &["--user", "--group", "--groups", "--name"],
                false,
            ),
            "ps" => args
                .iter()
                .all(|arg| matches!(*arg, "-e" | "-f" | "-ef" | "-elf" | "aux")),
            "systemctl" => {
                pager = true;
                systemctl(args)
            }
            "journalctl" => {
                pager = true;
                journal_limit = !journal(args)?;
                true
            }
            _ => false,
        };
        if !accepted {
            return None;
        }

        // Retain the PTY's current directory. Use fixed executable paths and
        // an empty child environment so aliases, functions, and configured
        // pagers cannot change the meaning of an approved diagnostic.
        let mut shell = Zeroizing::new(String::from(
            "LD_PRELOAD= LD_LIBRARY_PATH= LD_AUDIT= /usr/bin/env -i PATH=/usr/bin:/bin ",
        ));
        if !requested.starts_with('/') {
            shell.push_str("/usr/bin/");
        }
        shell.push_str(requested);
        if pager {
            shell.push_str(" --no-pager");
        }
        if journal_limit {
            shell.push_str(" --lines=100");
        }
        for argument in args.iter().filter(|arg| !pager || **arg != "--no-pager") {
            let argument = Zeroizing::new(argument.replace('\'', "'\"'\"'"));
            write!(&mut *shell, " '{}'", argument.as_str()).ok()?;
        }
        check_command(&shell).ok()?;
        Some(Self(shell))
    }

    pub(crate) fn wire(self) -> Zeroizing<String> {
        self.0
    }
}

fn simple_flags(args: &[&str], short: &str, long: &[&str], paths: bool) -> bool {
    let mut operands = false;
    args.iter().all(|&arg| {
        if !operands && arg == "--" {
            operands = true;
            return paths;
        }
        if let Some(flag) = arg.strip_prefix('-').filter(|_| !operands) {
            return long.contains(&arg)
                || !flag.is_empty() && flag.chars().all(|letter| short.contains(letter));
        }
        paths && !arg.is_empty()
    })
}

fn unit_name(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.@:-".contains(&byte))
}

fn systemctl(args: &[&str]) -> bool {
    let mut args = args.iter().copied().filter(|arg| *arg != "--no-pager");
    matches!(
        args.next(),
        Some("status" | "is-active" | "is-enabled" | "is-failed")
    ) && args.all(unit_name)
}

fn journal(args: &[&str]) -> Option<bool> {
    let bounded_count = |value: &str| {
        !value.is_empty()
            && value.bytes().all(|byte| byte.is_ascii_digit())
            && value
                .parse::<u16>()
                .is_ok_and(|count| (1..=500).contains(&count))
    };
    let mut args = args.iter().copied();
    let mut limited = false;
    while let Some(arg) = args.next() {
        match arg {
            "--no-pager" | "-b" | "--boot" | "--utc" => {}
            "-u" | "--unit" if args.next().is_some_and(unit_name) => {}
            "-n" | "--lines" if args.next().is_some_and(bounded_count) => limited = true,
            arg if arg.strip_prefix("--unit=").is_some_and(unit_name) => {}
            arg if arg.strip_prefix("--lines=").is_some_and(bounded_count) => limited = true,
            _ => return None,
        }
    }
    Some(limited)
}

#[cfg(test)]
mod tests {
    use super::ReadOnlyCommand;

    #[test]
    fn accepts_bounded_diagnostics_and_quotes_paths() {
        let list = ReadOnlyCommand::prepare("ls -lah '/opt/example logs'")
            .unwrap()
            .wire();
        assert!(list.ends_with("/usr/bin/ls '-lah' '/opt/example logs'"));
        let logs = ReadOnlyCommand::prepare("journalctl -u demo.service")
            .unwrap()
            .wire();
        assert!(logs.ends_with("/usr/bin/journalctl --no-pager --lines=100 '-u' 'demo.service'"));
        assert!(ReadOnlyCommand::prepare("systemctl --no-pager is-active demo.service").is_some());
    }

    #[test]
    fn leaves_compound_interactive_and_mutating_operations_outside_the_grant() {
        for command in [
            "ls; pwd",
            "ls | wc -l",
            r#"ls "$HOME""#,
            "ls *.log",
            "ls > output.txt",
            "systemctl restart demo.service",
            "journalctl --follow",
            "journalctl -n 501",
            "env ls",
            "ls\npwd",
            "./ls",
            "cd /tmp",
            "df --sync",
        ] {
            assert!(ReadOnlyCommand::prepare(command).is_none(), "{command}");
        }
    }
}
