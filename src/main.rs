use clap::{arg, App};
use std::{fs, process, path::Path, io::Read, thread, sync::{mpsc::{self, Receiver, Sender}, Arc}};
use regex::{Regex};
use reqwest::{self, header::{HeaderName, HeaderValue}};
use url::{Url};
use pbr::ProgressBar;
use serde_json;

#[derive(Debug)]
struct M3U8 {
    path: String,
    base_url: String,
    output: String,
    header: String,
    resume: bool
}

impl M3U8 {
    fn new(path: &str, base_url: &str, output: &str, header: &str, resume: bool) -> M3U8 {
        M3U8 {
            path: String::from(path),
            base_url: String::from(base_url),
            output: String::from(output),
            header: String::from(header),
            resume: resume
        }
    }

    fn parse_header(&self) -> Vec<(String, String)> {
        let mut headers: Vec<(String, String)> = vec![];
        if self.header.is_empty() {
            headers
        } else {
            let f = fs::read_to_string(&self.header).unwrap();
            let v: serde_json::Value = serde_json::from_str(&f).unwrap();
            match v {
                serde_json::Value::Object(ref map) => {
                    for (k, v) in map {
                        if let serde_json::Value::String(ref s) = v {
                            headers.push((String::from(k), String::from(s)));
                        }
                    }
                },
                _ => ()
            }
            println!("header: {:?}", headers);
            headers
        }
    }

    fn load_m3u8(path: &str) -> Vec<String> {
        let content = fs::read_to_string(path).unwrap_or_else(|op| {
            println!("File {:?} read failed, error: {}", path, op);
            process::exit(0);
        });

        let mut list = vec![];
        let ts_match = Regex::new(r"[a-zA-Z0-9]+\.ts").unwrap();
        for line in content.lines() {
            // Get encrypt key file.
            if let Some(pos) = ts_match.find(line) {
                list.push(String::from(pos.as_str()));
            }
        }
        list
    }

    fn download_ts(base_url: &str, list: &[String], tx: &Sender<(String, Vec<u8>)>, header: &[(String, String)])
    {
        let client = reqwest::blocking::Client::new();

        for ts in list {
            let url = Url::parse(base_url).unwrap().join(&ts).unwrap();
            let mut body = client.get(url.as_str());
            for h in header {
                body = body.header(HeaderName::from_bytes(&h.0.as_bytes()).unwrap(), 
                             HeaderValue::from_bytes(&h.1.as_bytes()).unwrap());
            }

            // deal with the http request error, try our best to download more ts files.
            match body.send() {
                Err(e) => {
                    println!("ts: {} download failed, error: {}", &ts, e);
                    continue;
                },
                Ok(resp) => {
                    if let Ok(body) = resp.bytes() {
                        let content: Result<Vec<_>, _> = body.bytes().collect();
                        if let Ok(data) = content {
                            tx.send((String::from(ts), data)).unwrap();
                        }
                    } else {
                        println!("ts: {} download failed, parse error", &ts);
                        continue;
                    }
                }
            }
        }
    }

    pub fn check_exist(&self, ts: &str) -> bool {
        let root_path = Path::new(&self.output);
        return root_path.join(&ts).exists();
    }

    pub fn download(&self, thread_num: i32) {
        if !Path::new(&self.output).exists() {
            fs::create_dir_all(&self.output).unwrap();
        }
        let mut list = M3U8::load_m3u8(&self.path);
        if list.len() == 0 {
            println!("m3u8 format is invalid");
            process::exit(0);
        }

        // Don't download the downloaded file if the file already existed.
        if self.resume {
            list = list.iter()
                    .filter(|ts| !self.check_exist(ts))
                    .map(|ts| ts.to_owned())
                    .collect();
        }

        if list.len() == 0{
           println!("Done!");
           process::exit(0);
        }

        let mut thread_pool: Vec<thread::JoinHandle<_>> = vec![];
        let iter = list.chunks(list.len() / (thread_num as usize));
        let (tx, rx): (Sender<(String, Vec<u8>)>, Receiver<(String, Vec<u8>)>) = mpsc::channel();
        let output_ref = self.output.clone();
        let total = list.len();
        thread_pool.push(thread::spawn( move || {
            let mut pb = ProgressBar::new(total as u64);
            for data in rx {
                let path = Path::new(&output_ref).join(&data.0);
                fs::write(path, data.1).unwrap();
                pb.inc();
            }
            pb.finish_print("Done!");
        }));
        
        let base_url = Arc::new(self.base_url.clone());
        let header = Arc::new(self.parse_header());
        for i in iter {
            let data = i.to_vec();
            let tx = tx.clone();
            let base_url = Arc::clone(&base_url);
            let header = Arc::clone(&header);
            thread_pool.push(thread::spawn( move || {
                M3U8::download_ts(&base_url, &data, &tx, &header);
            }));
        }

        drop(tx);

        for t in thread_pool {
            t.join().unwrap();
        }
    }
}

fn main() {
    let matches = App::new("Soo!")
        .version("1.0")
        .author("XBlame <xblame@qq.com>")
        .about("Multi-thread m3u8 downloader")
        .arg(arg!(-f --file <FILE> "the local path of the m3u8 file").required(true))
        .arg(arg!(-u --url  <URL> "the url of the m3u8 file").required(true))
        .arg(arg!(-d --dest <DIR> "the path of output dir").required(false))
        .arg(arg!(-j --j <N> "multi-thread number, default: 8").required(false))
        .arg(arg!(--header <JSON_FILE> "http request header, you can input a json file to declare it.").required(false))
        .arg(arg!(-r --resume "resume from break-point").required(false).takes_value(false))
        .get_matches();

    let file_path = matches.value_of("file").unwrap();
    let url = matches.value_of("url").unwrap();
    let dest = matches.value_of("dest").unwrap_or("./");
    let thread_num: i32 = matches.value_of_t("j").unwrap_or(8);
    let header = matches.value_of("header").unwrap_or("");
    let resume = matches.is_present("resume");

    let config: M3U8 = M3U8::new(file_path,url, dest, header, resume);
    config.download(thread_num);
}
