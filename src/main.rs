extern crate lzma;
extern crate gnuplot;
extern crate docopt;
#[macro_use]
extern crate serde_derive;
use docopt::Docopt;
use gnuplot::{Figure, Caption, Color};
use std::fs;
use std::env;
use std::io::BufRead;

const USAGE: &'static str = "
Solar Power Ploter

Usage:
  __PROGNAME__ <logdir> [-o GRAPH] [--avg=<sec>]
  __PROGNAME__ (-h | --help)

Options:
  -h --help     Show this screen.
  -o GRAPH      Plot file name [default: out.pdf]
  --avg=<sec>   Take the average of 'sec' seconds [default: 300]
";

#[derive(Debug, Deserialize)]
struct Args {
    arg_logdir: String,
    flag_o: String,
    flag_avg: i32,
}

struct Record {
    timestamp: f64,
    timezone: i8,
    current: f64,
    voltage: f64,
}

impl Record {
    fn new(line: &str) -> Option<Record> {
        let values = line.split(";").collect::<Vec<&str>>();

        if values.len() != 3 {
            None
        } else {
            let mut timestamp: f64 = 0.0;
            let mut current: f64 = 0.0;
            let mut voltage: f64 = 0.0;

            match values[0].parse::<f64>() {
                Ok(x) => timestamp = x,
                Err(_) => return None,
            }

            match values[1].parse::<f64>() {
                Ok(x) => current = x,
                Err(_) => return None,
            }

            match values[2].parse::<f64>() {
                Ok(x) => voltage = x,
                Err(_) => return None,
            }

        Some (Record {
            timestamp: timestamp,
            timezone: -2,
            current: current,
            voltage: voltage,
        })
        }
    }
}

fn parse_file(file: &str, recs: &mut Vec<Record>) {
    let decompressed = lzma::decompress(&fs::read(file).unwrap()).expect("Corrupt xz file!");
    let file_reader = std::io::BufReader::new(decompressed.as_slice());

    for (index, line) in file_reader.lines().enumerate() {
        match Record::new(&line.unwrap()) {
            Some(x) => recs.push(x),
            None => eprintln!("Malformed line: {}:{}", file, index + 1),
        }
    }
}

fn main() {
    /*
     * Parse arguments
     */
    let usage = USAGE.replace("__PROGNAME__", &env::args().nth(0).unwrap());
    let args: Args = Docopt::new(usage).and_then(|d| d.deserialize()).unwrap_or_else(|e| e.exit());

    println!("{:?}", args);
    let log_path = fs::read_dir(args.arg_logdir).expect("Directory not accessible.");

    let logs = log_path.map(|entry| {
		let entry = entry.unwrap();
		let entry_path = entry.path();
		let file_name = entry_path.to_str().unwrap();
		let file_name_as_string = String::from(file_name);
		file_name_as_string
	}).collect::<Vec<String>>();

    let mut recs = Vec::new();

    for file in logs {
        if file.ends_with(".log.xz") {
            println!("Filename: {}", &file);
            parse_file(&file, &mut recs);
        }
    }

    println!("Records: {}", recs.len());

    let x = [0u32, 1, 2];
    let y = [3u32, 4, 5];
    let mut fg = Figure::new();
    fg.axes2d()
        .lines(&x, &y, &[Caption("A line"), Color("blue")]);
    fg.set_terminal("pdfcairo", "out.pdf");
    fg.show();
}
