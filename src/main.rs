use std::{env, fs, process};

use video_prepare::{
    import_manual_visual_asset, parse_script, run_desktop, ProjectStore,
};

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None => launch_desktop(),
        Some("gui") => {
            if args.next().is_some() {
                usage();
            }
            launch_desktop();
        }
        Some("validate") => {
            let Some(path) = args.next() else {
                usage();
            };
            if args.next().is_some() {
                usage();
            }
            let input = read_script(&path);
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
        Some("create-project") => {
            let (Some(data_root), Some(project_id), Some(path)) =
                (args.next(), args.next(), args.next())
            else {
                usage();
            };
            if args.next().is_some() {
                usage();
            }
            let input = read_script(&path);
            let prepared = match parse_script(&input) {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("{}: {}", error.code(), error);
                    process::exit(1);
                }
            };
            let store = ProjectStore::new(data_root);
            match store.create(&project_id, &input, &prepared) {
                Ok(project) => {
                    println!("CREATED");
                    println!("project_id: {}", project.metadata.project_id);
                    println!("root: {}", project.root.display());
                    println!("input_sha256: {}", project.metadata.input_sha256);
                }
                Err(error) => {
                    eprintln!("PROJECT_CREATE_ERROR: {error}");
                    process::exit(1);
                }
            }
        }
        Some("open-project") => {
            let Some(project_root) = args.next() else {
                usage();
            };
            if args.next().is_some() {
                usage();
            }
            match ProjectStore::open(project_root) {
                Ok(project) => {
                    println!("OPENED");
                    println!("project_id: {}", project.metadata.project_id);
                    println!("title: {}", project.metadata.title);
                    println!("scenes: {}", project.metadata.scene_ids.len());
                    println!("status: {:?}", project.status.overall);
                }
                Err(error) => {
                    eprintln!("PROJECT_OPEN_ERROR: {error}");
                    process::exit(1);
                }
            }
        }
        Some("import-visual") => {
            let (Some(project_root), Some(scene_id), Some(visual_id), Some(source_path)) =
                (args.next(), args.next(), args.next(), args.next())
            else {
                usage();
            };
            if args.next().is_some() {
                usage();
            }
            let mut project = match ProjectStore::open(&project_root) {
                Ok(project) => project,
                Err(error) => {
                    eprintln!("PROJECT_OPEN_ERROR: {error}");
                    process::exit(1);
                }
            };
            match import_manual_visual_asset(&mut project, &scene_id, &visual_id, &source_path) {
                Ok(summary) => {
                    println!(
                        "{}",
                        if summary.already_present {
                            "ALREADY_PRESENT"
                        } else {
                            "IMPORTED"
                        }
                    );
                    println!("project_id: {}", summary.project_id);
                    println!("scene_id: {}", summary.scene_id);
                    println!("visual_id: {}", summary.visual_id);
                    println!("slot: {}", summary.slot);
                    println!("kind: {:?}", summary.kind);
                    println!("path: {}", summary.relative_path);
                    println!("sha256: {}", summary.sha256);
                    println!("bytes: {}", summary.bytes);
                    println!("request_state: {:?}", summary.request_state);
                    println!("scene_state: {:?}", summary.scene_state);
                    println!("visual_flow: {:?}", summary.flow_state);
                    println!("overall: {:?}", summary.overall);
                }
                Err(error) => {
                    eprintln!("MANUAL_VISUAL_IMPORT_ERROR: {error}");
                    process::exit(1);
                }
            }
        }
        _ => usage(),
    }
}

fn launch_desktop() {
    if let Err(error) = run_desktop() {
        eprintln!("DESKTOP_ERROR: {error}");
        process::exit(1);
    }
}

fn read_script(path: &str) -> String {
    match fs::read_to_string(path) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("SCRIPT_READ_ERROR: {path}: {error}");
            process::exit(1);
        }
    }
}

fn usage() -> ! {
    eprintln!("Usage:");
    eprintln!("  video-prepare [gui]");
    eprintln!("  video-prepare validate <script.vprep>");
    eprintln!("  video-prepare create-project <data-root> <project-id> <script.vprep>");
    eprintln!("  video-prepare open-project <project-root>");
    eprintln!("  video-prepare import-visual <project-root> <scene-id> <visual-id> <image-or-video-file>");
    process::exit(2);
}