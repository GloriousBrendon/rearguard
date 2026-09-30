// SPDX-License-Identifier: MIT OR Apache-2.0

//! `rearguard-server`: the Rearguard server (task 1.6). Loopback only until TLS and
//! authentication (task 3.3).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rearguard_server::config::Config;
use rearguard_server::log::Logger;
use rearguard_server::server::Server;
use rearguard_server::store::Store;
use rearguard_server::{create_master_secret, load_master_secret};

const USAGE: &str = "\
Usage:
  rearguard-server gen-secret PATH              Create a master secret file (must not exist)
  rearguard-server run --config FILE            Serve until Ctrl-C
  rearguard-server verdict --db FILE --session ID
                                                Print a stored session verdict as JSON

Relative paths in the config file are resolved from the config file's directory.
The master secret and session seeds are never printed or logged.";

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("rearguard-server: {message}");
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["gen-secret", path] => match create_master_secret(Path::new(path)) {
            Ok(()) => {
                println!("rearguard-server: wrote a new master secret to {path}");
                ExitCode::SUCCESS
            }
            Err(e) => fail(format_args!("{path}: {e}")),
        },
        ["run", "--config", path] => run(Path::new(path)),
        ["verdict", "--db", db, "--session", id] => verdict(Path::new(db), id),
        ["-h" | "--help"] => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn resolve(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
}

fn run(config_path: &Path) -> ExitCode {
    let text = match std::fs::read_to_string(config_path) {
        Ok(t) => t,
        Err(e) => return fail(format_args!("{}: {e}", config_path.display())),
    };
    let mut config = match Config::from_json(&text) {
        Ok(c) => c,
        Err(e) => return fail(format_args!("{}: {e}", config_path.display())),
    };
    let base = config_path.parent().unwrap_or(Path::new("."));
    config.master_secret_file = resolve(base, &config.master_secret_file);
    config.database = resolve(base, &config.database);
    let root = match load_master_secret(&config.master_secret_file) {
        Ok(r) => r,
        Err(e) => {
            return fail(format_args!(
                "master secret {}: {e}",
                config.master_secret_file.display()
            ));
        }
    };
    let store = match Store::open(&config.database) {
        Ok(s) => s,
        Err(e) => return fail(format_args!("database {}: {e}", config.database.display())),
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    let result = runtime.block_on(async move {
        let server = Server::bind(config, root, store, Logger::stderr()).await?;
        server
            .run(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(e),
    }
}

fn verdict(db: &Path, id: &str) -> ExitCode {
    let Ok(session_id) = id.parse::<u64>() else {
        return fail(format_args!("session id '{id}' is not a number"));
    };
    let store = match Store::open(db) {
        Ok(s) => s,
        Err(e) => return fail(format_args!("{}: {e}", db.display())),
    };
    match store.verdict(session_id) {
        Ok(Some(v)) => match serde_json::to_string_pretty(&v) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        },
        Ok(None) => fail(format_args!("no stored verdict for session {session_id}")),
        Err(e) => fail(e),
    }
}
