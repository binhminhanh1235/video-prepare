use std::{env, fs, process};

use video_prepare::parse_script;

fn main() {
    let mut args = env::args().skip(1);
    let command = args.next();
    let path = args.next();

    if command.as_deref() != Some("validate") || path.is_none() || args.next().is_some() {
        eprintln!("Usage: video-prepare validate <script.vprep>");
        process::exit(2);
    }

    let path = path.expect("path checked above");
    let input = match fs::read_to_string(&path) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("SCRIPT_READ_ERROR: {path}: {error}");
            process::exit(1);
        }
    };

    match parse_script(&input) {
        Ok(script) => {
            println!("VALID");
            println!("title: {}", script.omnivoice.title);
            println!("scenes: {}", script.scenes.len());
            println!("sections: {}", script.omnivoice.sections.len());
            println!("input_sha256: {}", script.input_sha256);
        }
        Err(error) => {
            eprintln!("{}: {}", error.code(), error);
            process::exit(1);
        }
    }
}
