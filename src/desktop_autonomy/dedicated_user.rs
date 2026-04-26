use super::*;
impl DesktopAutonomyManager {
    pub(super) fn apply_shadow_database_env(
        &self,
        command: &mut Command,
        database: &DatabaseConfig,
    ) {
        command.env("DATABASE_BACKEND", database.backend.to_string());
        match database.backend {
            crate::config::DatabaseBackend::Postgres => {
                command.env("DATABASE_URL", database.url());
                command.env("DATABASE_POOL_SIZE", database.pool_size.to_string());
            }
            crate::config::DatabaseBackend::LibSql => {
                if let Some(path) = database.libsql_path.as_ref() {
                    command.env("LIBSQL_PATH", path);
                }
                if let Some(url) = database.libsql_url.as_ref() {
                    command.env("LIBSQL_URL", url);
                }
                if let Some(token) = database.libsql_auth_token.as_ref() {
                    command.env("LIBSQL_AUTH_TOKEN", token.expose_secret());
                }
            }
        }
    }

    pub(super) fn shadow_binary_path(&self, build_dir: &Path) -> PathBuf {
        let exe = if cfg!(windows) {
            "thinclaw.exe"
        } else {
            "thinclaw"
        };
        build_dir.join("target").join("debug").join(exe)
    }

    pub(super) async fn user_exists(&self, username: &str) -> Result<bool, String> {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => match run_cmd(
                Command::new("dscl")
                    .arg(".")
                    .arg("-read")
                    .arg(format!("/Users/{username}")),
            )
            .await
            {
                Ok(_) => Ok(true),
                Err(err) if err.contains("eDSUnknownNodeName") => Ok(false),
                Err(err) => Err(err),
            },
            DesktopBridgeBackend::WindowsPowerShell => run_cmd(
                Command::new("cmd")
                    .arg("/C")
                    .arg(format!("net user {username}")),
            )
            .await
            .map(|_| true)
            .or_else(|err| {
                if err.contains("The user name could not be found") {
                    Ok(false)
                } else {
                    Err(err)
                }
            }),
            DesktopBridgeBackend::LinuxPython => {
                run_cmd(Command::new("id").arg("-u").arg(username))
                    .await
                    .map(|_| true)
                    .or_else(|err| {
                        if err.contains("no such user") {
                            Ok(false)
                        } else {
                            Err(err)
                        }
                    })
            }
            DesktopBridgeBackend::Unsupported => Ok(false),
        }
    }

    pub(super) async fn has_privileged_bootstrap(&self) -> bool {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift | DesktopBridgeBackend::LinuxPython => run_cmd(
                Command::new("id").arg("-u"),
            )
            .await
            .map(|uid| uid.trim() == "0")
            .unwrap_or(false),
            DesktopBridgeBackend::WindowsPowerShell => run_cmd(
                Command::new("powershell")
                    .arg("-NoLogo")
                    .arg("-NoProfile")
                    .arg("-Command")
                    .arg(
                        "[bool](([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator))",
                    ),
            )
            .await
            .map(|value| value.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false),
            DesktopBridgeBackend::Unsupported => false,
        }
    }

    pub(super) async fn create_dedicated_user(
        &self,
        username: &str,
        password: &str,
    ) -> Result<(), String> {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => {
                run_cmd(
                    Command::new("sysadminctl")
                        .arg("-addUser")
                        .arg(username)
                        .arg("-password")
                        .arg(password)
                        .arg("-home")
                        .arg(PathBuf::from("/Users").join(username)),
                )
                .await?;
                Ok(())
            }
            DesktopBridgeBackend::WindowsPowerShell => {
                let escaped_password = password.replace('\'', "''");
                run_cmd(
                    Command::new("powershell")
                        .arg("-NoLogo")
                        .arg("-NoProfile")
                        .arg("-Command")
                        .arg(format!(
                            "$pw = ConvertTo-SecureString '{escaped_password}' -AsPlainText -Force; \
                             New-LocalUser -Name '{username}' -Password $pw -AccountNeverExpires -PasswordNeverExpires; \
                             Add-LocalGroupMember -Group 'Users' -Member '{username}'"
                        )),
                )
                .await?;
                Ok(())
            }
            DesktopBridgeBackend::LinuxPython => {
                let user = shell_single_quote(username);
                let secret = shell_single_quote(password);
                run_cmd(
                    Command::new("sh")
                        .arg("-lc")
                        .arg(format!(
                            "useradd --create-home --shell /bin/bash {user} && \
                             printf '%s:%s\\n' {user} {secret} | chpasswd && \
                             for group in audio video input; do \
                               if getent group \"$group\" >/dev/null 2>&1; then usermod -aG \"$group\" {user}; fi; \
                             done"
                        )),
                )
                .await?;
                Ok(())
            }
            DesktopBridgeBackend::Unsupported => {
                Err("dedicated-user creation is unsupported on this platform".to_string())
            }
        }
    }

    pub(super) async fn gui_session_ready(
        &self,
        session_subject: &str,
        username: Option<&str>,
    ) -> bool {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => {
                if run_cmd(
                    Command::new("launchctl")
                        .arg("print")
                        .arg(format!("gui/{session_subject}")),
                )
                .await
                .is_ok()
                {
                    return true;
                }

                let Some(username) = username else {
                    return false;
                };
                run_cmd(
                    Command::new("stat")
                        .arg("-f")
                        .arg("%Su")
                        .arg("/dev/console"),
                )
                .await
                .map(|owner| owner.trim() == username)
                .unwrap_or(false)
            }
            DesktopBridgeBackend::WindowsPowerShell => {
                let user = username.unwrap_or(session_subject);
                run_cmd(Command::new("query").arg("user").arg(user))
                    .await
                    .is_ok()
            }
            DesktopBridgeBackend::LinuxPython => {
                let expected_user = username.unwrap_or(session_subject);
                let current_user = std::env::var("USER")
                    .ok()
                    .or_else(|| std::env::var("LOGNAME").ok())
                    .unwrap_or_default();
                let current_session_ready = (std::env::var_os("DISPLAY").is_some()
                    || std::env::var_os("WAYLAND_DISPLAY").is_some())
                    && current_user == expected_user;
                if current_session_ready {
                    return true;
                }
                let quoted_user = shell_single_quote(expected_user);
                run_cmd(
                    Command::new("sh")
                        .arg("-lc")
                        .arg(format!(
                            "command -v loginctl >/dev/null 2>&1 && \
                             loginctl list-sessions --no-legend 2>/dev/null | \
                             awk -v user={quoted_user} '$3 == user {{ found=1 }} END {{ exit found ? 0 : 1 }}'"
                        )),
                )
                .await
                .is_ok()
            }
            DesktopBridgeBackend::Unsupported => false,
        }
    }
}
