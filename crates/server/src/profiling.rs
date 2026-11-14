use kiarni_storage::profiling::{self, Metric};

pub static PUT_VALIDATION: Metric = Metric::new();
pub static PUT_ADMISSION: Metric = Metric::new();

pub(crate) const FLUSH_STACK_BYTES: usize = 32 * 1024;
pub(crate) const FLUSH_BUFFER_BYTES: usize = 8 * 1024;

pub fn emit() {
    use std::{
        fs::File,
        io::{self, Write},
        os::fd::AsFd,
        sync::mpsc,
        thread,
        time::Duration,
    };
    let bytes = format!(
        "KIARNI_PROFILE {}\n",
        serde_json::json!({
            "scope": "put_including_warmup",
            "put_validation": PUT_VALIDATION.snapshot(),
            "put_admission": PUT_ADMISSION.snapshot(),
            "storage_begin": profiling::BEGIN.snapshot(),
        "lookup": profiling::LOOKUP.snapshot(),
            "transform": profiling::TRANSFORM.snapshot(),
            "stage": profiling::STAGE.snapshot(),
            "commit": profiling::COMMIT.snapshot(),
        })
    )
    .into_bytes();
    if bytes.capacity() > FLUSH_BUFFER_BYTES {
        return;
    }
    let Ok(fd) = rustix::io::dup(io::stderr().as_fd()) else {
        return;
    };
    let (done, finished) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("kiarni-profile-flush".into())
        .stack_size(FLUSH_STACK_BYTES)
        .spawn(move || {
            let _ = File::from(fd).write_all(&bytes);
            let _ = done.send(());
        });
    if let Ok(worker) = worker
        && finished.recv_timeout(Duration::from_millis(50)).is_ok()
    {
        let _ = worker.join();
    }
}
