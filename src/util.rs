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

pub fn get_file_handler(path: &path::PathBuf, pre_alloc_size: u64) -> Result<File, std::io::Error> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .custom_flags(libc::O_DIRECT)
            .open(path)
            .expect("CRITICAL: Failed to open file with O_DIRECT. Verify FS supports it.");

        
        if let Ok(metadata) = file.metadata() {
            if metadata.len() < pre_alloc_size {
                 println!("Pre-allocating disk space...");
                 file.set_len(pre_alloc_size)?; 
                 // Force metadata sync to disk
                 file.sync_all()?; 
            }
        }
        Ok(file)
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
