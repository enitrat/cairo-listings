# Cairo Listings Verification Tool

A CLI tool designed to verify and validate Cairo programs, wrapping Cairo and Starknet plugins for quick verification of Cairo code examples.

## Features

- Compilation verification for Cairo programs and Starknet contracts
- Test execution and validation
- Code formatting checks
- Parallel processing of multiple packages
- Progress tracking with visual indicators
- Detailed error reporting and summaries

## Installation

```bash
cargo install cairo-listings
```

## Usage

The tool provides three main commands:

### 1. Verify Command

Runs comprehensive verification on Cairo programs:

```bash
cairo-listings verify [OPTIONS] <PATH>
```

Options:
- `--quiet`: Suppress progress output
- `--compile-skip`: Skip compilation checks
- `--run-skip`: Skip program execution checks
- `--test-skip`: Skip test execution
- `--formats-skip`: Skip format checking
- `--starknet-skip`: Skip Starknet contract compilation

### 2. Format Command

Runs formatting checks on Cairo files:

```bash
cairo-listings format [OPTIONS] <PATH>
```

### 3. Output Command

Process output files in listings:

```bash
cairo-listings output [OPTIONS]
```

## Verification Process

The tool performs several checks based on file content and tags:

1. **Compilation Checks**:
   - Standard Cairo programs are compiled
   - Starknet contracts are verified with `starknet-compile`

2. **Execution Checks**:
   - Runnable programs are executed with `cairo-run`
   - Tests are run with `cairo-test`

3. **Format Checks**:
   - Code formatting is verified unless explicitly ignored

## Special Tags

Files can include special tags at the top of Cairo files to control verification behavior. Tags should be placed in comments at the beginning of the file and can be combined using commas.

### Available Tags

- `does_not_compile`: Indicates that the code is intentionally non-compilable. The verification tool will skip compilation checks.
- `does_not_run`: Marks code that shouldn't be executed. The tool will skip runtime verification.
- `ignore_fmt`: Excludes the file from formatting checks.
- `tests_fail`: Indicates that the tests are expected to fail. The tool will skip test verification.

### Tag Usage Example

```cairo
// does_not_compile, tests_fail
fn example_function() {
    // This code intentionally doesn't compile and has failing tests
}
```

### Tag Combinations

Multiple tags can be combined in a single line, separated by commas:
```cairo
// does_not_compile, ignore_fmt, tests_fail
```

## Error Reporting

The tool provides:
- Detailed error messages with file locations
- Progress bars for bulk verification
- Comprehensive error summaries
- Clickable file paths in terminal output

## Exit Codes

- 0: All checks passed
- 1: One or more checks failed

## Useful Links

- [Cairo Book PR #209](https://github.com/cairo-book/cairo-book.github.io/pull/209)

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
```

This README provides a comprehensive overview of the tool's functionality based on the main.rs implementation, including all major features, commands, and options available in the CLI tool.
