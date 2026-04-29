use std::process::Command as StdCommand;

use tokio::process::Command as TokioCommand;

/// Supported shell launcher families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellFlavor {
    PosixSh,
    WindowsCmd,
}

/// Shared shell-launch helper for free-form commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellLauncher {
    flavor: ShellFlavor,
}

impl ShellLauncher {
    pub fn current() -> Self {
        #[cfg(target_os = "windows")]
        {
            Self {
                flavor: ShellFlavor::WindowsCmd,
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            Self {
                flavor: ShellFlavor::PosixSh,
            }
        }
    }

    pub fn flavor(self) -> ShellFlavor {
        self.flavor
    }

    pub fn program(self) -> &'static str {
        match self.flavor {
            ShellFlavor::PosixSh => "/bin/sh",
            ShellFlavor::WindowsCmd => "cmd",
        }
    }

    pub fn prefix_args(self) -> &'static [&'static str] {
        match self.flavor {
            ShellFlavor::PosixSh => &["-lc"],
            ShellFlavor::WindowsCmd => &["/C"],
        }
    }

    pub fn std_command(self, script: &str) -> StdCommand {
        let mut cmd = StdCommand::new(self.program());
        cmd.args(self.prefix_args()).arg(script);
        cmd
    }

    pub fn tokio_command(self, script: &str) -> TokioCommand {
        let mut cmd = TokioCommand::new(self.program());
        cmd.args(self.prefix_args()).arg(script);
        cmd
    }
}

pub fn shell_launcher() -> ShellLauncher {
    ShellLauncher::current()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_has_expected_prefix_args() {
        let launcher = shell_launcher();
        match launcher.flavor() {
            ShellFlavor::PosixSh => assert_eq!(launcher.prefix_args(), ["-lc"]),
            ShellFlavor::WindowsCmd => assert_eq!(launcher.prefix_args(), ["/C"]),
        }
    }
}
