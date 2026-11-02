use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::Output,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use colored::Colorize;
use glob::glob;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use regex::Regex;

use crate::{cmd::ScarbCmd, config::OutputArgs, utils::find_scarb_manifests};

/// Exit code of a Rust program that panicked.
const PANIC_EXIT_CODE: i32 = 101;

/// A command that crashed instead of producing an output, expected failure or not.
struct Crash {
    output_file: PathBuf,
    command: String,
    details: String,
}

pub fn process_outputs(arg: &OutputArgs) {
    let scarb_packages = find_scarb_manifests(arg.path.as_str());

    let total_packages = scarb_packages.len();
    let pb = Arc::new(ProgressBar::new(total_packages as u64));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
            .unwrap()
            .progress_chars("##-"),
    );

    let processed_count = Arc::new(AtomicUsize::new(0));
    let crashes = Mutex::new(Vec::new());

    scarb_packages.par_iter().for_each(|file| {
        process_file(file, &crashes);

        let current = processed_count.fetch_add(1, Ordering::SeqCst) + 1;
        pb.set_position(current as u64);

        if current == total_packages {
            pb.finish_with_message("Output processing complete");
        }
    });

    let crashes = crashes.into_inner().unwrap();
    if !crashes.is_empty() {
        report_crashes(&crashes);
        std::process::exit(1);
    }
}

/// Reads why a command failed, or `None` when its output can be written.
///
/// A non-zero exit is often the point of a listing: a program that does not
/// compile or a test that fails. A panic or a signal never is.
fn crash_details(result: &Result<Output, String>) -> Option<String> {
    let output = match result {
        Ok(output) => output,
        Err(e) => return Some(e.clone()),
    };
    match output.status.code() {
        Some(PANIC_EXIT_CODE) | None => Some(format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout).trim_end(),
            String::from_utf8_lossy(&output.stderr).trim_end()
        )),
        Some(_) => None,
    }
}

fn report_crashes(crashes: &[Crash]) {
    for crash in crashes {
        println!("\n{}", "=".repeat(80).red());
        println!(
            "{} running `{}` for {}",
            "Crash".red().bold(),
            crash.command.green(),
            crash.output_file.display().to_string().blue().underline()
        );
        for line in crash.details.lines() {
            println!("  {}", line);
        }
    }
    println!(
        "\n{}",
        format!(
            "{} command(s) crashed. Their output files were left unchanged.",
            crashes.len()
        )
        .red()
        .bold()
    );
}

fn process_file(manifest_path: &str, crashes: &Mutex<Vec<Crash>>) {
    // Find all output*.txt files in the manifest directory
    let glob_pattern = PathBuf::from(manifest_path)
        .parent()
        .expect("Failed to get parent directory")
        .join("output*.txt");

    let output_files = glob(glob_pattern.to_str().unwrap())
        .expect("Failed to read glob pattern")
        .filter_map(Result::ok)
        .collect::<Vec<PathBuf>>();

    for output_file_path in output_files {
        if let Some(command_with_args) = read_command_from_output_file(&output_file_path) {
            let (command, args) = command_with_args;
            let cmd = match command.as_str() {
                "scarb build" => ScarbCmd::Build(),
                "scarb cairo-run" => ScarbCmd::CairoRun(),
                "scarb test" | "scarb cairo-test" => ScarbCmd::Test(),
                "scarb format" => ScarbCmd::Format(),
                "scarb execute" => ScarbCmd::Execute(),
                "scarb verify" => ScarbCmd::Verify(),
                "scarb prove" => ScarbCmd::Prove(),
                _ => {
                    eprintln!("Unknown command in {:?}: {}", output_file_path, command);
                    continue;
                }
            };

            let result = cmd.run(manifest_path, args.clone());
            if let Some(details) = crash_details(&result) {
                crashes.lock().unwrap().push(Crash {
                    output_file: output_file_path,
                    command: format!("{} {}", command, args.join(" ")),
                    details,
                });
                continue;
            }
            let mut output = String::from_utf8_lossy(&result.unwrap().stdout).into_owned();

            let re2 = Regex::new(r"-->\s*.*?(listings/.*)").expect("Failed to create regex");
            output = re2.replace_all(&output, "--> $1").to_string();

            // Remove the line that contains "Saving output to"
            let re3 = Regex::new(r"Saving output to.*").expect("Failed to create regex");
            output = re3.replace_all(&output, "").to_string();

            // Replace all the `<whatever_comes_here>/cairo-book/listings/` with `listings/`
            let re4 = Regex::new(r"/(.*?)/cairo-book/listings/").expect("Failed to create regex");
            output = re4.replace_all(&output, "listings/").to_string();

            let re5 = Regex::new(r"Finished `dev` profile target\(s\) in .* second(s)?").expect("Failed to create regex");
            output = re5.replace_all(&output, "Finished `dev` profile target(s) in 1 second").to_string();

            // Remove the `Blocking waiting for file lock on package cache` lines
            output = output
                .lines()
                .filter(|line| !line.contains("Blocking waiting for file lock on package cache"))
                .collect::<Vec<&str>>()
                .join("\n");

            // Keep the command in the first line of the output file
            let to_write = format!("$ {} {}\n{}\n\n", command, args.join(" "), output);

            if let Err(e) = write_output_file(&output_file_path, &to_write) {
                eprintln!("Failed to write output file {:?}: {}", output_file_path, e);
            }
        }
    }
}

fn read_command_from_output_file(output_file_path: &PathBuf) -> Option<(String, Vec<String>)> {
    let file = File::open(output_file_path).ok()?;
    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    reader.read_line(&mut first_line).ok()?;

    if let Some(full_command) = first_line.strip_prefix("$ ") {
        let mut parts = full_command.split_whitespace();
        let command = match (parts.next(), parts.next()) {
            (Some(cmd), Some(subcmd)) => format!("{} {}", cmd, subcmd),
            (Some(cmd), None) => cmd.to_string(),
            _ => return None,
        };

        let args = parts.map(|arg| arg.to_string()).collect();

        Some((command, args))
    } else {
        None
    }
}

fn write_output_file(output_file_path: &PathBuf, content: &str) -> std::io::Result<()> {
    let mut file = File::create(output_file_path)?;
    file.write_all(content.as_bytes())?;
    Ok(())
}
