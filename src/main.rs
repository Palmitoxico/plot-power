extern crate lzma;
extern crate gnuplot;
extern crate docopt;
extern crate chrono;
#[macro_use]
extern crate serde_derive;
use docopt::Docopt;
use gnuplot::{Figure, Caption, Color};
use std::fs;
use std::env;
use std::io::BufRead;
use std::sync::{Mutex, Arc};
use std::cmp::Ordering;
use std::thread;
use gnuplot::*;
use chrono::prelude::*;

const USAGE: &'static str = "
Solar Power Ploter

Usage:
  __PROGNAME__ <logdir> [-o OUTDIR] [--avg=<sec>] [-t THREADS] [--time-zone=<tz>]
  __PROGNAME__ (-h | --help)

Options:
  -h --help         Show this screen.
  -o OUTDIR         Plot file name [default: ./]
  --avg=<sec>       Take the average of 'sec' seconds [default: 300]
  -t THREADS        Number of threads for processing the input data [default: 1]
  --time-zone=<tz>  Specify the timezone [default: 0];
";

#[derive(Debug, Deserialize)]
struct Args {
    arg_logdir: String,
    flag_o: String,
    flag_avg: i32,
    flag_t: i32,
    flag_time_zone: i32,
}

#[derive (Clone)]
struct Record {
    timestamp_ms: i64,
    current: f32,
    voltage: f32,
}

impl Ord for Record {
    fn cmp(&self, other: &Record) -> Ordering {
        self.timestamp_ms.cmp(&other.timestamp_ms)
    }
}

impl PartialOrd for Record {
    fn partial_cmp(&self, other: &Record) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Record {
    fn eq(&self, other: &Record) -> bool {
        self.timestamp_ms == other.timestamp_ms
    }
}

impl Eq for Record {}

impl Record {
    fn new(line: &str) -> Option<Record> {
        let values = line.split(";").collect::<Vec<&str>>();

        if values.len() != 3 {
            None
        } else {

            let timestamp_ms: i64 =
                match values[0].parse::<f64>() {
                    Ok(x) => (x * 1000.0) as i64,
                    Err(_) => return None,
                };

            let current: f32 =
                match values[1].parse::<f32>() {
                    Ok(x) => x,
                    Err(_) => return None,
                };

            let voltage: f32 =
                match values[2].parse::<f32>() {
                    Ok(x) => x,
                    Err(_) => return None,
                };


            if timestamp_ms < 1_000_000_000_000 || timestamp_ms > 2_000_000_000_000 {
                return None;
            } else if current < -15.0 || current > 15.0 {
                return None;
            } else if voltage < 5.0 || voltage > 16.0 {
                return None;
            }

        Some (Record {
            timestamp_ms: timestamp_ms,
            current: current,
            voltage: voltage,
        })
        }
    }
}

struct Dispatcher {
    files: Vec<String>,
    job_index: usize,
    recs: Vec<Record>,
}

impl Dispatcher {
    fn new() -> Dispatcher {
        Dispatcher {
            files: Vec::new(),
            job_index: 0,
            recs: Vec::new(),
        }
    }

    fn set_log_dir(&mut self, path: &str) {
        let log_path = fs::read_dir(path).expect("Directory not accessible.");
        let logs = log_path.map(|entry| {
		    let entry = entry.unwrap();
		    let entry_path = entry.path();
		    let file_name = entry_path.to_str().unwrap();
		    let file_name_as_string = String::from(file_name);
		    file_name_as_string
	    }).collect::<Vec<String>>();
        for file in logs {
            if file.ends_with(".log.xz") {
                self.files.push(file);
            }
        }
    }

    fn get_next_job(&mut self) -> Option<String> {
        let j_idx = self.job_index;
        if j_idx >= self.files.len() {
            None
        } else {
            self.job_index += 1;
            Some(self.files[j_idx].clone())
        }
    }

    fn append_data(&mut self, data: &Vec<Record>){
        self.recs.extend_from_slice(data.as_slice());
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

fn take_avg(recs: &Vec<Record>, start_index: usize, delta_time_ms: i64) -> (usize, Record) {
    let start_time = recs[start_index].timestamp_ms;
    let mut rec_index = start_index;
    let mut avg_index: i64 = 0;
    let mut current_acc: f32 = 0.0;
    let mut voltage_acc: f32 = 0.0;

    for rec in &recs[start_index..] {
        if (rec.timestamp_ms - start_time) >= delta_time_ms {
            break;
        }
        current_acc += rec.current;
        voltage_acc += rec.voltage;
        avg_index += 1;
        rec_index += 1;
    }

    (rec_index, Record {
        timestamp_ms: start_time,
        current: current_acc / if avg_index > 0 {(avg_index as f32)} else {1.0},
        voltage: voltage_acc / if avg_index > 0 {(avg_index as f32)} else {1.0},
    })
}

fn main() {
    /*
     * Parse arguments
     */
    let usage = USAGE.replace("__PROGNAME__", &env::args().nth(0).unwrap());
    let args: Args = Docopt::new(usage).and_then(|d| d.deserialize()).unwrap_or_else(|e| e.exit());

    let disp_mut = Arc::new(Mutex::new(Dispatcher::new()));

    {
        let mut disp = disp_mut.lock().unwrap();
        disp.set_log_dir(&args.arg_logdir);
        if disp.files.len() == 0 {
            eprintln!("Error: directory '{}' doesn't contain log files.", &args.arg_logdir);
            std::process::exit(2);
        }
    }

    let mut thread_vec = Vec::new();

    /*
     * Create n threads to decompress and parse the log files
     */
    for _ in 0..args.flag_t {
        let disp_mut = Arc::clone(&disp_mut);
        let t = thread::spawn(move || {
            loop {
                let file =
                {
                    let mut disp = disp_mut.lock().unwrap();
                    match disp.get_next_job() {
                        Some(x) => x,
                        None => break,
                    }
                };
                let mut recs = Vec::new();
                println!("Filename: {}", &file);
                parse_file(&file, &mut recs);
                {
                    let mut disp = disp_mut.lock().unwrap();
                    disp.append_data(&recs);
                }
            }
        });
        thread_vec.push(t);
    }

    /*
     * Wait for all threads to finish
     */
    for t in thread_vec {
        t.join().unwrap();
    }

    let mut avg_recs: Vec<Record> = Vec::new();
    {
        let mut disp = disp_mut.lock().unwrap();
        println!("Records: {}", disp.recs.len());
        println!("Sorting...");
        disp.recs.sort();
        println!("Averaging [{} seconds]...", args.flag_avg);
        let mut next_index: usize = 0;
        loop {
            let res = take_avg(&disp.recs, next_index, (args.flag_avg as i64) * 1000);
            next_index = res.0;
            avg_recs.push(res.1);
            if (next_index + 20) >= disp.recs.len() {
                break;
            }
        }
        println!("Records avg count: {}", avg_recs.len());
        disp.recs.clear();
    }

    println!("Ploting...");
    let mut rec_index: usize = 0;

    /*
     * Generate one plot per day
     */
    loop {
        if rec_index >= avg_recs.len() {
            break;
        }
        let mut x_time: Vec<f32> = Vec::new();
        let mut y_power: Vec<f32> = Vec::new();
        let time_zone = FixedOffset::east(args.flag_time_zone * 3600);
        let start_date = NaiveDateTime::from_timestamp(avg_recs[rec_index].timestamp_ms / 1000, 0);
        let start_date: DateTime<FixedOffset> = time_zone.from_utc_datetime(&start_date);
        let start_day = start_date.day();

        let mut count: usize = 0;
        for rec in &avg_recs[rec_index..] {
            let actual_date = NaiveDateTime::from_timestamp(rec.timestamp_ms / 1000, 0);
            let actual_date: DateTime<FixedOffset> = time_zone.from_utc_datetime(&actual_date);
            if start_day != actual_date.day() {
                break;
            }
            x_time.push(actual_date.hour() as f32 + actual_date.minute() as f32 / 60.0);
            y_power.push(rec.current * rec.voltage);
            count += 1;
        }
        rec_index += count;

        let date_str = start_date.format("%Y-%m-%d").to_string();
        let time_zone_str = start_date.format(" GMT%z").to_string();
        let mut fg = Figure::new();
        let mut title = String::new();
        title.push_str("Power Plot - ");
        title.push_str(&date_str);
        title.push_str(&time_zone_str);
        fg.axes2d()
            .set_title(&title, &[])
            .lines(&x_time, &y_power, &[Caption("Power"), Color("blue")])
            .set_x_label("Day hour", &[])
            .set_y_label("Power [W]", &[])
            .set_grid_options(true, &[LineStyle(DotDotDash), Color("gray")])
		    .set_x_grid(true)
		    .set_y_grid(true);

        let mut filename = String::new();
        filename.push_str(&args.flag_o);
        filename.push_str("/");
        filename.push_str(&date_str);
        filename.push_str(".pdf");
        fg.set_terminal("pdfcairo", &filename);
        fg.show();
    }
    println!("Done!");
}
