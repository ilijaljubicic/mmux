use clap::Parser;
use mmux_microsandbox_node::{
    launch, logs, resume, snapshot, snapshot_export, snapshot_import, status, stop, Cli, Command,
};
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Launch(args) => launch(args).await.map(|report| {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        }),
        Command::Snapshot(args) => snapshot(args).await.map(|report| {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        }),
        Command::SnapshotExport(args) => snapshot_export(args).await.map(|report| {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        }),
        Command::SnapshotImport(args) => snapshot_import(args).await.map(|report| {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
        }),
        Command::Status(args) => status(args).await.map(|value| {
            println!("{}", serde_json::to_string_pretty(&value).unwrap());
        }),
        Command::Resume(args) => resume(args).await.map(|_| {
            println!("resumed");
        }),
        Command::Stop(args) => stop(args).await.map(|_| {
            println!("stopped");
        }),
        Command::Logs(args) => logs(args).await.map(|value| {
            print!("{}", value);
        }),
    };

    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mmux-microsandbox-node error: {}", error);
            ExitCode::from(1)
        }
    }
}
