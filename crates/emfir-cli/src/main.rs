use clap::{Parser, Subcommand};
use mrc::MrcFile;
use eer::{show_header_info, generate_thumbnail};
use std::path::PathBuf;
use std::process;

#[derive(Parser)]
#[command(name = "emfir-cli")]
#[command(about = "CLI for handling MRC and EER data", long_about = None)]
struct Cli {
    /// Path to the input file
    #[arg(short, long)]
    file: PathBuf,

    /// Command: either "header" or "thumbnail"
    #[arg(short, long)]
    command: String,
    
    /// Output path for thumbnail (required for thumbnail command)
    #[arg(short, long)]
    output: Option<PathBuf>,
    
    /// Downsample factor for thumbnail generation (process every Nth frame)
    #[arg(short, long, default_value = "10")]
    downsample: u32,
}

fn main() {
    let cli = Cli::parse();

    let extension = cli.file
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");

    match extension {
        "mrc" => {
            match MrcFile::open(&cli.file.to_string_lossy()) {
                Ok(mrc) => {

                    let image_data = mrc.get_image_data();
                    match serde_json::to_string_pretty(image_data) {
                        Ok(json) => println!("{}", json),
                        Err(e) => {
                            eprintln!("Error serializing to JSON: {}", e);
                            process::exit(1);
                        }
                    }
        
                    // // Generate thumbnail if requested
                    // if let Some(thumbnail_path) = args.thumbnail {
                    //     if let Err(e) = mrc.save_thumbnail(&thumbnail_path, args.downsample) {
                    //         eprintln!("Error generating thumbnail: {}", e);
                    //         process::exit(1);
                    //     }
                    // }
                }
                Err(err) => {
                    eprintln!("Error reading MRC file: {}", err);
                    process::exit(1);
                }
            }
        }
        "eer" => {
             match cli.command.as_str() {
                "header" => {
                    show_header_info(&cli.file);
                },
                "thumbnail" => {
                    if let Some(output_path) = &cli.output {
                        match generate_thumbnail(&cli.file, output_path, Some(cli.downsample)) {
                            Ok(_) => println!("Thumbnail generated at {:?}", output_path),
                            Err(e) => {
                                eprintln!("Error generating thumbnail: {}", e);
                                process::exit(1);
                            }
                        }
                    } else {
                        eprintln!("Output path is required for thumbnail command. Use --output");
                        process::exit(1);
                    }
                },
                _ => {
                    eprintln!("Unknown command: {}. Use 'header' or 'thumbnail'.", cli.command);
                }
            }
        }
        _ => {
            eprintln!("Can't handle file with this extension: {}", extension);
        }
    }

    // match cli.command.as_str() {
    //     "header" => {
    //         if is_mrc {
    //             // call MRC code
    //             println!("Reading MRC header from {:?}", cli.file);
    //             // mrc::print_header(&cli.file);
    //         } else if is_eer {
    //             println!("Reading EER header from {:?}", cli.file);
    //             // eer::print_header(&cli.file);
    //         } else {
    //             eprintln!("Unknown file type for {:?}", cli.file);
    //         }
    //     },
    //     "thumbnail" => {
    //         if is_mrc {
    //             println!("Generating MRC thumbnail for {:?}", cli.file);
    //             // mrc::generate_thumbnail(&cli.file);
    //         } else if is_eer {
    //             println!("Generating EER thumbnail for {:?}", cli.file);
    //             // eer::generate_thumbnail(&cli.file);
    //         } else {
    //             eprintln!("Unknown file type for {:?}", cli.file);
    //         }
    //     },
    //     _ => {
    //         eprintln!("Unknown command: {}. Use 'header' or 'thumbnail'.", cli.command);
    //     }
    // }
}
