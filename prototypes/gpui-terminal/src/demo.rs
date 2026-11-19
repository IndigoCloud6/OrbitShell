use orbitshell_ssh::ConnectionProfile;

pub struct Profile {
    pub title: String,
    pub address: String,
    pub group: usize,
    pub home: &'static str,
    pub remote: Option<ConnectionProfile>,
    pub temporary: bool,
}

impl Profile {
    pub fn from_ssh(config: ConnectionProfile, group: usize) -> Self {
        let host = if config.hostname.contains(':') {
            format!("[{}]", config.hostname)
        } else {
            config.hostname.clone()
        };
        let address = if config.port == 22 {
            format!("{}@{host}", config.username)
        } else {
            format!("{}@{host}:{}", config.username, config.port)
        };
        Self {
            title: config.name.clone(),
            address,
            group,
            home: "",
            remote: Some(config),
            temporary: false,
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Directory {
    #[default]
    Home,
    Config,
    Logs,
    Releases,
}

impl Directory {
    pub fn name(self) -> &'static str {
        match self {
            Self::Home => "",
            Self::Config => "config",
            Self::Logs => "logs",
            Self::Releases => "releases",
        }
    }

    pub fn entries(self) -> &'static [Entry] {
        const HOME: &[Entry] = &[
            Entry::directory(Directory::Config, "config"),
            Entry::directory(Directory::Logs, "logs"),
            Entry::directory(Directory::Releases, "releases"),
            Entry::file("README.md", "1.2 KB"),
            Entry::file("compose.yaml", "2.4 KB"),
            Entry::file("deploy.sh", "874 B"),
            Entry::file(".env.example", "24 B"),
        ];
        const CONFIG: &[Entry] = &[
            Entry::file("app.toml", "743 B"),
            Entry::file("logging.yaml", "1.0 KB"),
        ];
        const LOGS: &[Entry] = &[
            Entry::file("service.log", "24 KB"),
            Entry::file("startup.log", "3.1 KB"),
        ];
        const RELEASES: &[Entry] = &[
            Entry::file("current.json", "452 B"),
            Entry::file("manifest.txt", "1.8 KB"),
        ];

        match self {
            Self::Home => HOME,
            Self::Config => CONFIG,
            Self::Logs => LOGS,
            Self::Releases => RELEASES,
        }
    }

    pub fn path(self, home: &str) -> String {
        if self == Self::Home {
            home.to_owned()
        } else {
            format!("{home}/{}", self.name())
        }
    }
}

pub struct Entry {
    pub name: &'static str,
    pub size: &'static str,
    pub opens: Option<Directory>,
}

impl Entry {
    pub fn contents(&self) -> &'static str {
        match self.name {
            "README.md" => {
                "# OrbitShell 示例工作区\n\nconfig/    应用配置\nlogs/      服务日志\nreleases/  发布清单\n"
            }
            "compose.yaml" => {
                "services:\n  gateway:\n    image: example.test/gateway:demo\n    ports:\n      - \"8080:8080\"\n"
            }
            "deploy.sh" => "#!/bin/sh\n# 本地预览文件\nprintf '%s\\n' 'OrbitShell demo'\n",
            ".env.example" => "APP_ENV=demo\nAPP_PORT=8080\n",
            "app.toml" => "[server]\nlisten = \"127.0.0.1:8080\"\nmode = \"demo\"\n",
            "logging.yaml" => "logging:\n  level: info\n  format: text\n",
            "service.log" | "startup.log" => {
                "INFO  configuration loaded\nINFO  local fixture initialized\n"
            }
            "current.json" => "{\n  \"release\": \"demo\",\n  \"channel\": \"preview\"\n}\n",
            "manifest.txt" => "gateway\nconfig/app.toml\nREADME.md\n",
            _ => "",
        }
    }

    const fn directory(opens: Directory, name: &'static str) -> Self {
        Self {
            name,
            size: "—",
            opens: Some(opens),
        }
    }

    const fn file(name: &'static str, size: &'static str) -> Self {
        Self {
            name,
            size,
            opens: None,
        }
    }
}
