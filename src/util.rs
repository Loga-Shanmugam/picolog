use std::fs::{File, OpenOptions};
use std::path;

pub fn get_blksize(path: &path::PathBuf) -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(metadata) = std::fs::metadata(path) {
            use std::os::linux::fs::MetadataExt;
            return metadata.st_blksize();
        } else {
            return 4096; // Default fallback
        }
    }

    #[cfg(target_os = "windows")]
    {
        //TODO: Implement Windows procedure
        panic!("Windows block size detection not implemented");
    }

    #[cfg(target_os = "macos")]
    {
        //TODO: Implement mac procedure
        panic!("MacOS block size detection not implemented");
    }
}

pub fn get_file_handler(path: &path::PathBuf) -> Result<File, std::io::Error> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let res = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .custom_flags(libc::O_DIRECT)
            .open(path);
        
        if res.is_err() {
             println!("O_DIRECT failed, using default buffered IO ");
             return OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path);
        }
        println!("Using O_DIRECT");
        res
    }
    #[cfg(target_os = "windows")]
    {
        //TODO: Implement Windows equivalent of IO-uring
        panic!("Windows file handler with direct IO not implemented");
    }
    #[cfg(target_os = "macos")]
    {
        //TODO: Implement MacOS equivalent of IO-uring
        panic!("MacOS file handler with direct IO not implemented");
    }
}
