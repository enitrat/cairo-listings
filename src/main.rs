use cairo_lang_parser::utils::SimpleParserDatabase;
use clap::Parser;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[macro_use]
extern crate lazy_static;

mod cmd;
mod config;
mod error_sets;
mod logger;
mod output;
mod tags;
mod utils;

use crate::cmd::ScarbCmd;
use crate::config::{Commands, Config, VerifyArgs};
use crate::error_sets::ErrorSets;
use crate::tags::Tags;
use crate::utils::{clickable, find_scarb_manifests, print_error_table};

lazy_static! {
    static ref ERRORS: Mutex<ErrorSets> = Mutex::new(ErrorSets::new());
}

lazy_static! {
    static ref CFG: Config = Config::parse();
}

fn main() {
    let cfg = Config::parse();

    match &cfg.command {
        Commands::Verify(args) => run_verification(args),
        Commands::Output(args) => output::process_outputs(args),
        Commands::Format(args) => run_format(args),
    }
}

fn run_verification(args: &VerifyArgs) {
    let scarb_packages = find_scarb_manifests(args.path.as_str());

    let total_packages = scarb_packages.len();
    let pb = Arc::new(ProgressBar::new(total_packages as u64));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
            .unwrap()
            .progress_chars("##-"),
    );

    let processed_count = Arc::new(AtomicUsize::new(0));

    logger::setup(args, Arc::clone(&pb));

    let args = Arc::new(args);

    scarb_packages.par_iter().for_each(|file| {
        let args = Arc::clone(&args);
        process_file(file, &args);

        if !args.quiet {
            let current = processed_count.fetch_add(1, Ordering::SeqCst) + 1;
            pb.set_position(current as u64);

            if current == total_packages {
                pb.finish_with_message("Verification complete");
            }
        }
    });

    let errors = ERRORS.lock().unwrap();
    print_error_summary(&errors);

    if errors.compile_errors.len()
        + errors.run_errors.len()
        + errors.test_errors.len()
        + errors.format_errors.len()
        > 0
    {
        std::process::exit(1);
    }
}

fn run_format(args: &VerifyArgs) {
    let scarb_packages = find_scarb_manifests(args.path.as_str());

    let total_packages = scarb_packages.len();
    let pb = Arc::new(ProgressBar::new(total_packages as u64));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
            .unwrap()
            .progress_chars("##-"),
    );

    let processed_count = Arc::new(AtomicUsize::new(0));

    logger::setup(args, Arc::clone(&pb));

    let args = Arc::new(args);

    scarb_packages.par_iter().for_each(|file| {
        process_package_files_format(file, &args, false);

        if !args.quiet {
            let current = processed_count.fetch_add(1, Ordering::SeqCst) + 1;
            pb.set_position(current as u64);

            if current == total_packages {
                pb.finish_with_message("Verification complete");
            }
        }
    });

    let errors = ERRORS.lock().unwrap();
    let total_errors = errors.format_errors.len();

    if total_errors > 0 {
        println!("{}\n", "  ==== RESULT ===  ".red().bold());

        print_error_table(&errors.format_errors, "Format Errors");

        println!(
            "{}",
            format!("Total errors: {}", total_errors.to_string().red()).bold()
        );

        println!("\n{}", "Please review the errors above. Do not hesitate to ask for help by commenting on the issue on Github.".red().italic());
        std::process::exit(1);
    } else {
        println!("\n{}\n", "FORMATTING COMPLETED!".green().bold());
    }
}

fn get_executable_targets(manifest_path: &str) -> Vec<String> {
    let manifest_path_as_path = std::path::Path::new(manifest_path);
    let manifest_file = File::open(manifest_path_as_path).unwrap();
    let manifest_reader = BufReader::new(manifest_file);
    let manifest_contents = manifest_reader.lines().collect::<Result<Vec<String>, _>>().unwrap();

    let mut executable_names = Vec::new();
    let mut in_executable_section = false;

    for line in manifest_contents {
        let line = line.trim();

        // Check if we're entering an executable section
        if line == "[[target.executable]]" || line == "[executable]" {
            in_executable_section = true;
            continue;
        }

        // Check if we're leaving the section (entering a new section)
        if line.starts_with('[') && in_executable_section {
            in_executable_section = false;
            continue;
        }

        // If we're in an executable section, look for the name
        if in_executable_section && line.starts_with("name = ") {
            if let Some(name) = line.strip_prefix("name = ") {
                // Remove quotes if present
                let name = name.trim_matches('"').trim_matches('\'');
                executable_names.push(name.to_string());
            }
        }
    }

    executable_names
}

fn process_file(manifest_path: &str, args: &VerifyArgs) {
    let manifest_path_as_path = std::path::Path::new(manifest_path);
    let executable_targets = get_executable_targets(manifest_path);
    let file_path = manifest_path_as_path
        .parent()
        .unwrap()
        .join("src/lib.cairo");
    let file_path = file_path.to_str().unwrap();

    let file = match File::open(file_path) {
        Ok(f) => f,
        Err(_) => {
            println!(
                "{}",
                format!("Warning: Failed to open file {}", file_path).yellow()
            );
            return;
        }
    };
    let reader = BufReader::new(file);

    // Parsed tags (if any)
    let mut tags: HashSet<Tags> = HashSet::new();
    let mut in_tag_block = true;

    // Program info
    let mut should_be_runnable = false;
    let mut should_be_testable = false;
    let mut is_contract = false;

    reader.lines().for_each(|line| {
        if let Ok(line_contents) = line {
            // Parse tags
            if in_tag_block && config::TAG_REGEX.is_match(&line_contents) {
                let line_contents = config::TAG_REGEX.replace(&line_contents, "");
                let tags_in_line: Vec<&str> = line_contents
                    .trim()
                    .split(',')
                    .map(|tag| tag.trim())
                    .collect();

                tags_in_line.iter().for_each(|tag| {
                    if let Some(tag_enum) = tags::Tags::from_str(tag) {
                        tags.insert(tag_enum);
                    }
                });
            } else {
                // Stop parsing tags when we reach the first non-comment line
                in_tag_block = false;
            }

            // Check for statements
            is_contract |= line_contents.contains(config::STATEMENT_IS_CONTRACT);
            should_be_runnable |= line_contents.contains(config::STATEMENT_IS_RUNNABLE);
            should_be_testable |= line_contents.contains(config::STATEMENT_IS_TESTABLE);
            should_be_testable |= line_contents.contains(config::STATEMENT_TEST_MODULE);
        }
    });

    // COMPILE / RUN CHECKS
    if is_contract {
        // This is a contract, it must pass starknet-compile
        if !tags.contains(&Tags::DoesNotCompile) && !args.starknet_skip {
            run_command(ScarbCmd::Build(), manifest_path, file_path, vec![], true);
        }
    } else if should_be_runnable {
        // This is a cairo program, it must pass cairo-run
        if !tags.contains(&Tags::DoesNotRun)
            && !tags.contains(&Tags::DoesNotCompile)
            && !args.run_skip
        {
            if !executable_targets.is_empty() {
                // Execute each executable target
                for executable_name in &executable_targets {
                    run_command(
                        ScarbCmd::Execute(),
                        manifest_path,
                        file_path,
                        vec!["--executable-name".to_string(), executable_name.clone()],
                        true,
                    );
                }
            } else {
                run_command(
                    ScarbCmd::CairoRun(),
                    manifest_path,
                    file_path,
                    vec!["--available-gas=200000000".to_string()],
                    true,
                );
            }
        }
    } else {
        // This is a cairo program, it must pass cairo-compile
        if !tags.contains(&Tags::DoesNotCompile) && !args.compile_skip {
            run_command(ScarbCmd::Build(), manifest_path, file_path, vec![], true);
        }
    };

    // TEST CHECKS
    if should_be_testable && !args.test_skip && !tags.contains(&Tags::FailingTests) {
        // This program has tests, it must pass cairo-test
        let _ = run_command(ScarbCmd::Test(), manifest_path, file_path, vec![], true);
    }

    // FORMAT CHECKS
    if !tags.contains(&Tags::IgnoreFormat) && !args.formats_skip {
        process_package_files_format(manifest_path, args, true);
    }
}

fn collect_cairo_files(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_cairo_files(&path, files);
            } else if path.extension().and_then(|s| s.to_str()) == Some("cairo") {
                files.push(path);
            }
        }
    }
}


/// Formats all .cairo files in the package
/// # Arguments
/// * `manifest_path`: The path to the manifest file
/// * `args`: The arguments passed to the program
/// * `check_only`: Whether to check only or format
fn process_package_files_format(manifest_path: &str, args: &VerifyArgs, check_only: bool) {
    let manifest_path_as_path = std::path::Path::new(manifest_path);
    let base_dir = manifest_path_as_path
        .parent()
        .unwrap();

    // Collect all .cairo files from src and tests directories
    let mut cairo_files = Vec::new();
    let src_dir = base_dir.join("src");
    if src_dir.exists() {
        collect_cairo_files(&src_dir, &mut cairo_files);
    }
    let tests_dir = base_dir.join("tests");
    if tests_dir.exists() {
        collect_cairo_files(&tests_dir, &mut cairo_files);
    }

    for file_path in cairo_files {
        let file_path = file_path.to_str().unwrap();
        let file = match File::open(file_path) {
            Ok(f) => f,
            Err(_) => {
                println!(
                    "{}",
                    format!("Warning: Failed to open file {}", file_path).yellow()
                );
                return;
            }
        };

        let reader = BufReader::new(file);

        // Parsed tags (if any)
        let mut tags: HashSet<Tags> = HashSet::new();
        let mut in_tag_block = true;

        reader.lines().for_each(|line| {
            if let Ok(line_contents) = line {
                // Parse tags
                if in_tag_block && config::TAG_REGEX.is_match(&line_contents) {
                    let line_contents = config::TAG_REGEX.replace(&line_contents, "");
                    let tags_in_line: Vec<&str> = line_contents
                        .trim()
                        .split(',')
                        .map(|tag| tag.trim())
                        .collect();

                    tags_in_line.iter().for_each(|tag| {
                        if let Some(tag_enum) = tags::Tags::from_str(tag) {
                            tags.insert(tag_enum);
                        }
                    });
                } else {
                    // Stop parsing tags when we reach the first non-comment line
                    in_tag_block = false;
                }
            }
        });

        // FORMAT CHECKS
        if !tags.contains(&Tags::IgnoreFormat) && !args.formats_skip {
            let code = std::fs::read_to_string(file_path).unwrap();
            let formatted_code = cairo_lang_formatter::format_string(&SimpleParserDatabase::default(), code.clone());
            if check_only {
                if code != formatted_code {
                    handle_error(format!("{} is not formatted correctly", file_path), file_path, ScarbCmd::Format(), args.verbose);
                }
            } else {
                std::fs::write(file_path, formatted_code).unwrap();
            }
        }
    }
}

fn run_command(
    cmd: ScarbCmd,
    manifest_path: &str,
    file_path: &str,
    args: Vec<String>,
    verbose: bool,
) -> String {
    match cmd.test(manifest_path, args) {
        Ok(output) => String::from_utf8_lossy(&output.stdout).into_owned(),
        Err(e) => handle_error(e, file_path, cmd, verbose),
    }
}

fn handle_error(e: String, file_path: &str, cmd: ScarbCmd, verbose: bool) -> String {
    let clickable_file = clickable(file_path);

    if verbose {
        println!("\n{}", "=".repeat(80).yellow());
        println!(
            "{} in {}",
            "Error".red().bold(),
            file_path.blue().underline()
        );
        println!("{}", "=".repeat(80).yellow());

        println!("{}:", "Command".cyan().bold());
        println!("  {}", cmd.as_str().green());

        println!("{}:", "File".cyan().bold());
        println!("  {}", clickable_file);

        println!("{}:", "Details".cyan().bold());
        for line in e.lines() {
            println!("  {}", line);
        }

        println!("{}", "=".repeat(80).yellow());
    }

    let mut errors = ERRORS.lock().unwrap();
    errors.get_mut_error_set(&cmd).insert(file_path.to_string());

    e
}

fn print_error_summary(errors: &ErrorSets) {
    let total_errors = errors.compile_errors.len()
        + errors.run_errors.len()
        + errors.test_errors.len()
        + errors.format_errors.len();

    if total_errors > 0 {
        println!("\n{}", "=".repeat(80).yellow());
        println!("{}", "  ERROR SUMMARY  ".red().bold().on_black());
        println!("{}", "=".repeat(80).yellow());

        print_error_category("Compile Errors", &errors.compile_errors);
        print_error_category("Run Errors", &errors.run_errors);
        print_error_category("Test Errors", &errors.test_errors);
        print_error_category("Format Errors", &errors.format_errors);

        println!("{}", "=".repeat(80).yellow());
        println!("  Total Errors: {}", total_errors.to_string().red().bold());
        println!("{}", "=".repeat(80).yellow());

        println!("\n{}", "Please review the errors above. If you need assistance, consider opening an issue on GitHub.".yellow().italic());
    } else {
        println!("\n{}", "=".repeat(80).green());
        println!("{}", "  ALL TESTS PASSED  ".green().bold().on_black());
        println!("{}", "=".repeat(80).green());
    }
}

fn print_error_category(category: &str, errors: &HashSet<String>) {
    if !errors.is_empty() {
        println!("\n{} ({})", category.cyan().bold(), errors.len());
        for error in errors {
            println!("  • {}", error);
        }
    }
}
