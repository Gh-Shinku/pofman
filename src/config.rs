use anyhow::{Context, Result, bail};
use directories::{BaseDirs, ProjectDirs};
use serde::Deserialize;
use std::{collections::HashSet, env, fs, path::PathBuf};

pub struct Paths {
    pub config: PathBuf,
    pub state: PathBuf,
}

impl Paths {
    pub fn ensure_config(&self) -> Result<()> {
        if let Some(parent) = self
            .config
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).context("Cannot create config directory")?;
        }
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.config)
        {
            Ok(_) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error).context("Cannot create config file"),
        }
    }

    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("", "", "pofman").context("Cannot locate user directories")?;
        Ok(Self {
            config: match env::var_os("POFMAN_CONFIG") {
                Some(path) => PathBuf::from(path),
                None => BaseDirs::new()
                    .context("Cannot locate home directory")?
                    .home_dir()
                    .join(".config")
                    .join("pofman")
                    .join("config.toml"),
            },
            state: env::var_os("POFMAN_STATE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| dirs.data_local_dir().join("run")),
        })
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub profiles: Vec<Profile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    pub command: String,
}

impl Config {
    pub fn load(paths: &Paths) -> Result<Self> {
        let text = match fs::read_to_string(&paths.config) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                paths.ensure_config()?;
                fs::read_to_string(&paths.config).context("Cannot read config")?
            }
            Err(e) => return Err(e).context("Cannot read config"),
        };
        Self::parse(&text).with_context(|| format!("Invalid config: {}", paths.config.display()))
    }

    fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text)?;
        let mut names = HashSet::new();
        for profile in &config.profiles {
            if profile.name.trim().is_empty() || profile.name.chars().any(char::is_control) {
                bail!("Profile names must be nonempty and contain no control characters");
            }
            if !names.insert(&profile.name) {
                bail!("Duplicate profile name: {}", profile.name);
            }
            if profile.command.trim().is_empty() {
                bail!("Empty command for {}", profile.name);
            }
        }
        Ok(config)
    }
}

impl Profile {
    pub fn argv(&self) -> Result<Vec<String>> {
        let args = shell_words::split(&self.command).context("Invalid command quoting")?;
        let executable = args.first().context("Empty SSH command")?;
        let name = std::path::Path::new(executable)
            .file_name()
            .and_then(|n| n.to_str());
        if !matches!(name, Some("ssh" | "ssh.exe")) {
            bail!("Profile {} must invoke ssh directly", self.name);
        }
        // Skip option values, and reject flags that fork or replace forwarding with a query.
        let mut args_iter = args.iter().skip(1);
        while let Some(arg) = args_iter.next() {
            if arg == "--" || !arg.starts_with('-') {
                break;
            }
            if let Some(options) = arg.strip_prefix('-') {
                for (index, option) in options.char_indices() {
                    if matches!(option, 'f' | 'O' | 'M' | 'S' | 'G' | 'Q' | 'V') {
                        bail!("SSH option -{option} is incompatible with managed forwarding");
                    }
                    if "BbcDEeFIiJLlmopPRSWw".contains(option) {
                        if index + option.len_utf8() == options.len() {
                            args_iter.next();
                        }
                        break;
                    }
                }
            }
        }
        Ok(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_config() {
        assert!(Config::parse("").unwrap().profiles.is_empty());
        assert!(Config::parse("[[profiles]]\nname='x'\ncommand='ssh host'\n[[profiles]]\nname='x'\ncommand='ssh host'").is_err());
        assert!(Config::parse("[[profiles]]\nname=''\ncommand='ssh host'").is_err());
        assert!(Config::parse("[[profiles]]\nname='x'\ncommand='ssh host'\ntypo=true").is_err());
    }
    #[test]
    fn parses_ssh_without_shell() {
        let profile = |command: &str| Profile {
            name: "test".into(),
            command: command.into(),
        };
        assert_eq!(
            profile("ssh -i 'key with spaces' -L 8080:localhost:80 host")
                .argv()
                .unwrap()[2],
            "key with spaces"
        );
        assert!(profile("ssh -Nf host").argv().is_err());
        assert!(profile("ssh -i file-with-f host").argv().is_ok());
        assert!(profile("sh -c 'ssh host'").argv().is_err());
    }
}
