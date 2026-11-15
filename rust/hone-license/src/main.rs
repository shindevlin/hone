//! `hone-license` — inspect or complete HONE's license acceptance step.
//!
//!   hone-license            same as `accept`
//!   hone-license accept     run the acceptance step (prompt, or flag/env)
//!   hone-license status     print whether this data dir has accepted
//!   hone-license show       print the full license text
//!   hone-license statement  print the UFL Section 1C Ruling License line
//!
//! Useful before starting HONE under a service manager: accept once
//! interactively here, and the record lets the service start headless.

use hone_license::{
    default_data_dir, read_record, record_path, require_acceptance_or_exit, RecordState,
    LICENSE_TEXT, RULING_LICENSE, SPDX,
};

fn main() {
    let args = hone_license::args();
    let data_dir = default_data_dir();
    match args.get(1).map(String::as_str).unwrap_or("accept") {
        "accept" => {
            let a = require_acceptance_or_exit(&data_dir);
            println!("accepted: {RULING_LICENSE} ({SPDX}), record {}", a.record_path.display());
        }
        "status" => {
            let path = record_path(&data_dir);
            match read_record(&path) {
                RecordState::Matching => println!("accepted: {RULING_LICENSE} ({})", path.display()),
                RecordState::Other { version, scope } => {
                    println!("not accepted: record is for UFL {version}, {scope}; HONE now requires {RULING_LICENSE} ({})", path.display());
                    std::process::exit(hone_license::EXIT_NOT_ACCEPTED);
                }
                RecordState::Missing | RecordState::Unreadable => {
                    println!("not accepted: no valid record at {}", path.display());
                    std::process::exit(hone_license::EXIT_NOT_ACCEPTED);
                }
            }
        }
        "show" => print!("{LICENSE_TEXT}"),
        "statement" => println!("{RULING_LICENSE} ({SPDX})"),
        other => {
            eprintln!("hone-license: unknown command {other:?} (accept | status | show | statement)");
            std::process::exit(2);
        }
    }
}
