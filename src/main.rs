use clap::Subcommand;
use jwalk::WalkDir;

use std::{
    error::Error,
    fs::{self, create_dir_all, rename, Metadata},
    path::{Path, PathBuf},
};

use clap::Parser;
use serde::{Deserialize, Serialize};
use std::os::unix::fs as unixFs;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    sub_command: SubCommands,
    #[arg()]
    manifest: String,
    #[clap(long, short, action)]
    prefix: Option<String>,
}

#[derive(Subcommand, Clone, Debug)]
enum SubCommands {
    Activate,
    Deactivate,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    files: Vec<File>,
    clobber_by_default: bool,
    version: u16,
}

#[derive(Serialize, Deserialize)]
struct File {
    source: String,
    target: String,
    recursive: Option<bool>,
    clobber: Option<bool>,
}

fn read_manifest(manifest: &str) -> Result<Manifest, Box<dyn Error>> {
    let raw_manifest = fs::read_to_string(manifest)?;
    let deserialized_manifest: Manifest = serde_json::from_str(&raw_manifest)?;
    Ok(deserialized_manifest)
}

fn mkdir(target: &str) -> Result<(), Box<dyn Error>> {
    match fs::symlink_metadata(target) {
        Err(_) => create_dir_all(target)?,
        Ok(x) => {
            if !x.is_dir() {
                return Err(format!("File in way of directory '{}'", target).into());
            } else {
                println!("Directory '{}' already exists...", target);
            };
        }
    };
    Ok(())
}

fn prefix_move(path: &str, prefix: &str) -> Result<(), Box<dyn Error>> {
    let as_path = Path::new(path);
    let new_path = format!(
        "{}-{}",
        prefix,
        as_path
            .file_name()
            .ok_or("Failed to get previous filename")?
            .to_str()
            .ok_or("Failed to turn path into string")?
    );

    if fs::symlink_metadata(&new_path).is_ok() {
        prefix_move(&new_path, prefix)?
    };

    rename(
        as_path,
        as_path
            .parent()
            .ok_or("Failed to get parent")?
            .join(new_path),
    )?;
    Ok(())
}
fn symlink(file: &File) -> Result<(), Box<dyn Error>> {
    unixFs::symlink(Path::new(&file.source), Path::new(&file.target))?;
    Ok(())
}
// How do I clean up residual symlinks
fn recursive_symlink(file: &File, prefix: &str, clobber: bool) -> Result<(), Box<dyn Error>> {
    fn resolve_link(link: PathBuf) -> Result<PathBuf, Box<dyn Error>> {
        Ok(if link.is_symlink() {
            resolve_link(fs::read_link(link)?)?
        } else {
            link
        })
    }

    let target_path = Path::new(&file.target);
    for entry in WalkDir::new(&file.source).follow_links(true) {
        match entry {
            Err(e) => {
                eprintln!(
                    "Recursive file walking error on base path: {}\n{}",
                    &file.source, e
                );
                continue;
            }
            Ok(ref x) => {
                let target_file = target_path.join(x.path().strip_prefix(&file.source)?);
                if x.path().is_dir() {
                    mkdir(
                        target_file
                            .to_str()
                            .ok_or("Failed to turn path into string")?,
                    )?;
                    continue;
                };

                if let Ok(x) = fs::symlink_metadata(&target_file) {
                    file_in_way(
                        target_file
                            .to_str()
                            .ok_or("Failed to turn path into string")?,
                        clobber,
                        prefix,
                        &x,
                    )?;
                }

                let source = if x.path_is_symlink() {
                    resolve_link(x.path())?
                } else {
                    x.path()
                };
                unixFs::symlink(source, &target_file)?;
            }
        };
    }
    Ok(())
}

fn file_in_way(
    path: &str,
    clobber: bool,
    prefix: &str,
    metadata: &Metadata,
) -> Result<(), Box<dyn Error>> {
    if clobber {
        if metadata.is_file() || metadata.is_symlink() {
            fs::remove_file(path)?;
        } else {
            Err(format!("Folder in way '{}'", path))?;
        };
    } else {
        prefix_move(path, prefix)?
    };
    Ok(())
}

fn activate(manifest: Manifest, prefix: String) {
    for file in manifest.files {
        if fs::symlink_metadata(&file.source).is_err() {
            eprintln!("File source '{}', does not exist", file.source,);
            continue;
        }

        let recursive = file.recursive.is_some_and(|x| x);
        let clobber = file.clobber.unwrap_or(manifest.clobber_by_default);

        match mkdir(
            Path::new(&file.target)
                .parent()
                .expect("Failed to get parent")
                .to_str()
                .expect("Failed to turn path into string"),
        ) {
            Ok(x) => x,
            Err(e) => eprintln!(
                "Couldn't create directory '{}'\n Reason: {}",
                file.target, e
            ),
        };

        if !recursive {
            if let Ok(x) = fs::symlink_metadata(&file.target) {
                if let Err(e) = file_in_way(&file.target, clobber, &prefix, &x) {
                    eprintln!("Failed to move file! {}\n'{}'", &file.target, e);
                };
            };
        };

        if let Err(e) = match recursive {
            true => recursive_symlink(&file, &prefix, clobber),
            false => symlink(&file),
        } {
            eprintln!("Failed to handle '{}'\nReason: {}", &file.source, e);
            continue;
        };
    }
}

fn deactivate(manifest: Manifest) {
    for file in manifest.files {
        if file.recursive.is_some_and(|x| x) {

        } else {
            let res = {
                let Ok(metdata) = fs::symlink_metadata(&file.target) else {
                    continue;
                };
                if metdata.is_file() || metdata.is_symlink() {
                    fs::remove_file(&file.target)
                } else {
                    fs::remove_dir_all(&file.target)
                }
            };

            if let Err(e) = res {
                eprintln!("Didn't cleanup file '{}'\nReason: {}", file.target, e)
            }
        };
    }
}

fn main() {
    const VERSION: u16 = 1;
    let args = Args::parse();

    let manifest = match read_manifest(&args.manifest) {
        Ok(x) => x,
        Err(e) => panic!("Failed to read or parse manifest!\n{}", e),
    };

    println!("Deserialized manifest: '{}'", args.manifest);
    println!("Manifest version: '{}'", manifest.version);
    println!("Program version: '{}'", VERSION);
    if manifest.version != VERSION {
        panic!("Version mismatch!\n Program and manifest version must be the same");
    };
    match args.sub_command {
        SubCommands::Deactivate => deactivate(manifest),
        SubCommands::Activate => activate(manifest, args.prefix.unwrap_or(".backup".to_string())),
    }
}
