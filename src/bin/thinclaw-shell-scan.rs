use std::io::Read;

use thinclaw::tools::builtin::shell_security::structural_external_scan;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut command_arg: Option<String> = None;
    let mut json_output = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json_output = true,
            "--command" => {
                command_arg = args.next();
            }
            "--help" | "-h" => {
                print_help();
                return;
            }
            other => {
                eprintln!("unknown argument: {other}");
                print_help();
                std::process::exit(2);
            }
        }
    }

    let command = if let Some(command) = command_arg {
        command
    } else {
        let mut stdin = String::new();
        if std::io::stdin().read_to_string(&mut stdin).is_err() {
            eprintln!("failed to read command from stdin");
            std::process::exit(2);
        }
        stdin
    };

    let report = structural_external_scan(&command);
    if json_output {
        println!(
            "{}",
            serde_json::to_string(&report).expect("external scan report should serialize")
        );
    } else if let Some(reason) = report.reason.as_deref() {
        println!("{reason}");
    } else {
        println!("safe");
    }

    match report.verdict {
        thinclaw::tools::builtin::shell_security::ExternalScanVerdict::Dangerous => {
            std::process::exit(1)
        }
        thinclaw::tools::builtin::shell_security::ExternalScanVerdict::Safe => {
            std::process::exit(0)
        }
        thinclaw::tools::builtin::shell_security::ExternalScanVerdict::Unknown => {
            std::process::exit(3)
        }
    }
}

fn print_help() {
    eprintln!(
        "Usage: thinclaw-shell-scan [--json] [--command <command>]\n\
         Reads a shell command from stdin when --command is omitted."
    );
}
