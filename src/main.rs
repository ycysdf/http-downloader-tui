use std::fmt::Write;
use std::io::stdout;
use std::num::{NonZeroU8, NonZeroUsize};
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::{arg, Parser};
use crossterm::cursor::{Hide, MoveToColumn, MoveToNextLine, MoveToPreviousLine};
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::{Clear, ClearType};
use http_downloader::{breakpoint_resume::DownloadBreakpointResumeExtension, HttpDownloaderBuilder, speed_limiter::DownloadSpeedLimiterExtension, speed_tracker::DownloadSpeedTrackerExtension};
use http_downloader::bson_file_archiver::{ArchiveFilePath, BsonFileArchiverBuilder};
use url::Url;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    url: Url,

    #[arg(short, long, default_value_t = NonZeroU8::new(3).unwrap())]
    connection_count: NonZeroU8,

    #[arg(long, default_value_t = NonZeroUsize::new(1024 * 1024 * 4).unwrap())]
    chunk_size: NonZeroUsize,

    #[arg(short, long, default_value = None)]
    speed_limit: Option<usize>,

    #[arg(short, long, default_value_t = true)]
    progress: bool,

    #[arg(long, default_value_t = false)]
    silence: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let result = std::env::current_exe()?;

    let save_dir = result.join("../");
    let (downloader, (speed_state, ..)) =
        HttpDownloaderBuilder::new(args.url, save_dir)
            .download_connection_count(args.connection_count)
            .chunk_size(args.chunk_size)
            .build((
                DownloadSpeedTrackerExtension { log: false },
                DownloadSpeedLimiterExtension {
                    byte_count_per: args.speed_limit
                },
                DownloadBreakpointResumeExtension {
                    download_archiver_builder: BsonFileArchiverBuilder::new(ArchiveFilePath::Suffix("bson".to_string()))
                }
            ));
    let file_path = downloader.get_file_path();
    execute!(
        stdout(),
        Hide
    )?;
    let mut is_first = true;
    let finished_future = downloader.start().await?;
    if !args.silence && args.progress {
        let mut bar = ProgressBar::new(62);
        tokio::spawn({
            let mut downloaded_len_receiver = downloader.downloaded_len_receiver().clone();
            async move {
                let total_len = downloader.total_size().await;
                while downloaded_len_receiver.changed().await.is_ok() {
                    let downloaded_len = *downloaded_len_receiver.borrow();
                    if let Some(total_len) = total_len {
                        let buf = bar.update(downloaded_len, total_len, speed_state.download_speed())?;
                        if is_first {
                            execute!(
                                stdout(),
                                Print(buf)
                            )?;
                            is_first = false;
                        } else {
                            execute!(
                                stdout(),
                                Clear(ClearType::CurrentLine),
                                MoveToPreviousLine(1),
                                Clear(ClearType::CurrentLine),
                                MoveToColumn(0),
                                Print(buf)
                            )?;
                        }
                    }

                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Result::<()>::Ok(())
            }
        });
    }

    let dec = finished_future.await?;

    if !args.silence {
        execute!(
        stdout(),
        MoveToNextLine(1),
        Clear(ClearType::CurrentLine),
        MoveToPreviousLine(1),
        Clear(ClearType::CurrentLine),
        MoveToColumn(0),
        Print(format!("{:?}",dec)),
    )?;
        println!("Save To: {}", file_path.display());
    }
    Ok(())
}


pub struct ProgressBar {
    bar_buf: String,
    buf: String,
    start_instant: Instant,
    bar_width: usize,
}

impl ProgressBar {
    pub fn new(max_width: usize) -> Self {
        Self {
            buf: String::new(),
            bar_buf: String::new(),
            start_instant: Instant::now(),
            bar_width: crossterm::terminal::size().ok()
                .map(|(cols, _rows)| usize::from(cols))
                .unwrap_or(0).min(max_width),
        }
    }
    fn update(&mut self, downloaded_len: u64, total_len: u64, speed: u64) -> Result<&str, std::fmt::Error> {
        let progress = (downloaded_len * 100 / total_len) as usize;

        let (downloaded_len_size, downloaded_len_unit) = Self::byte_unit(downloaded_len);
        let (total_len_size, total_len_unit) = Self::byte_unit(total_len);
        let (speed_size, speed_unit) = Self::byte_unit(speed);


        self.bar_buf.clear();
        self.buf.clear();
        let duration = self.start_instant.elapsed();
        write!(self.bar_buf, "{speed_size:.2} {speed_unit}/s - {progress} % - elapsed: {duration:.2?} ")?;
        write!(self.buf, "{downloaded_len_size:.2} {downloaded_len_unit} / {total_len_size:.2} {total_len_unit}")?;
        for _ in 0..(self.bar_width - self.bar_buf.len() - self.buf.len()) {
            self.bar_buf.push(' ');
        }
        writeln!(self.bar_buf, "{}", self.buf)?;

        let bar_p_width = self.bar_width - 2;
        let progress_width = progress * bar_p_width / 100;
        self.bar_buf.push('[');
        for _ in 0..progress_width {
            self.bar_buf.push('█');
        }
        for _ in progress_width..bar_p_width {
            self.bar_buf.push(' ');
        }
        self.bar_buf.push(']');

        Ok(&self.bar_buf)
    }


    fn byte_unit(bytes_count: u64) -> (f32, &'static str) {
        const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

        let mut i = 0;
        let mut bytes_count = bytes_count as f32;
        while bytes_count >= 1024.0 && i < UNITS.len() - 1 {
            i += 1;
            bytes_count /= 1024.0;
        }
        (bytes_count, UNITS[i])
    }
}