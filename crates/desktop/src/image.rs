//! Background image loading with a bounded worker queue and deferred admission.
//! Only encoded sources and decoded pixels cross threads; retained elements and
//! graph state never leave the UI thread.

use crate::renderer::{ImageId, RasterImage};
use refract_core::ImageSource;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    io::{Cursor, Read},
    rc::{Rc, Weak},
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

const MAX_ENCODED_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DIMENSION: u32 = 8192;
const QUEUE_CAPACITY: usize = 32;

pub(crate) enum ImageState {
    Loading,
    Ready(Arc<RasterImage>),
    Failed(String),
}

pub(crate) struct ImageResource {
    pub id: ImageId,
    pub state: ImageState,
}

pub(crate) type ImageHandle = Rc<RefCell<ImageResource>>;
type Completion = (ImageId, Result<RasterImage, String>);
struct Job {
    id: ImageId,
    source: ImageSource,
}

/// Entries share one request/resource for equal sources. Weak references allow
/// unmounted resources to be reclaimed, and discard late completions safely.
pub(crate) struct ImageStore {
    cache: HashMap<ImageSource, Weak<RefCell<ImageResource>>>,
    pending: HashMap<ImageId, Weak<RefCell<ImageResource>>>,
    jobs: Option<mpsc::SyncSender<Job>>,
    // Overflow stays on the UI thread until a worker completion frees capacity.
    // The pending weak reference determines whether a request is still needed.
    deferred: VecDeque<Job>,
    completed: mpsc::Receiver<Completion>,
    completion_sender: mpsc::Sender<Completion>,
    wake: Arc<dyn Fn() + Send + Sync>,
    next_id: u64,
    pub decoded: u64,
}

impl ImageStore {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (completion_sender, completed) = mpsc::channel();
        Self {
            cache: HashMap::new(),
            pending: HashMap::new(),
            jobs: None,
            deferred: VecDeque::new(),
            completed,
            completion_sender,
            wake: Arc::new(wake),
            next_id: 1,
            decoded: 0,
        }
    }

    fn start_workers(&mut self) {
        if self.jobs.is_some() {
            return;
        }
        let (jobs, receiver) = mpsc::sync_channel::<Job>(QUEUE_CAPACITY);
        self.spawn_workers(receiver);
        self.jobs = Some(jobs);
    }

    fn spawn_workers(&self, receiver: mpsc::Receiver<Job>) {
        let receiver = Arc::new(Mutex::new(receiver));
        for index in 0..2 {
            let receiver = receiver.clone();
            let completed = self.completion_sender.clone();
            let wake = self.wake.clone();
            // Workers exit when the host drops its sender. Never join on the UI
            // thread: an in-flight network read has its own finite timeout.
            std::thread::Builder::new()
                .name(format!("refract-image-{index}"))
                .spawn(move || {
                    let agent: ureq::Agent = ureq::Agent::config_builder()
                        .timeout_global(Some(Duration::from_secs(15)))
                        .build()
                        .into();
                    loop {
                        let job = receiver.lock().expect("image queue poisoned").recv();
                        let Ok(job) = job else {
                            break;
                        };
                        let result =
                            load(&agent, &job.source).and_then(|bytes| decode(job.id, &bytes));
                        if completed.send((job.id, result)).is_err() {
                            break;
                        }
                        wake();
                    }
                })
                .expect("could not start image loader thread");
        }
    }

    pub fn acquire(&mut self, source: &ImageSource) -> ImageHandle {
        self.cache.retain(|_, entry| entry.strong_count() != 0);
        if let Some(handle) = self.cache.get(source).and_then(Weak::upgrade) {
            return handle;
        }
        self.start_workers();
        let id = ImageId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("image identity exhausted");
        let handle = Rc::new(RefCell::new(ImageResource {
            id,
            state: ImageState::Loading,
        }));
        self.cache.insert(source.clone(), Rc::downgrade(&handle));
        self.pending.insert(id, Rc::downgrade(&handle));
        let job = Job {
            id,
            source: source.clone(),
        };
        // Preserve FIFO when older requests are already waiting for admission.
        if !self.deferred.is_empty() {
            self.deferred.push_back(job);
            return handle;
        }
        match self.jobs.as_ref().unwrap().try_send(job) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(job)) => self.deferred.push_back(job),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.pending.remove(&id);
                handle.borrow_mut().state = ImageState::Failed("image workers disconnected".into());
            }
        }
        handle
    }

    /// Publish worker results on the UI thread. Callers invalidate only elements
    /// still referencing the completed resource, never an old source or mount.
    pub fn poll(&mut self) -> Vec<ImageId> {
        let mut changed = Vec::new();
        while let Ok((id, result)) = self.completed.try_recv() {
            let Some(handle) = self.pending.remove(&id).and_then(|weak| weak.upgrade()) else {
                continue;
            };
            handle.borrow_mut().state = match result {
                Ok(image) => {
                    self.decoded += 1;
                    ImageState::Ready(Arc::new(image))
                }
                Err(error) => {
                    eprintln!("Refract image failed: {error}");
                    ImageState::Failed(error)
                }
            };
            changed.push(id);
        }
        while let Some(job) = self.deferred.pop_front() {
            let Some(handle) = self.pending.get(&job.id).and_then(Weak::upgrade) else {
                self.pending.remove(&job.id);
                continue;
            };
            match self.jobs.as_ref().unwrap().try_send(job) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(job)) => {
                    self.deferred.push_front(job);
                    // In-flight jobs will wake the event loop when they finish.
                    // Do not block or keep waking it while the queue is full.
                    break;
                }
                Err(mpsc::TrySendError::Disconnected(job)) => {
                    self.pending.remove(&job.id);
                    handle.borrow_mut().state =
                        ImageState::Failed("image workers disconnected".into());
                    changed.push(job.id);
                }
            }
        }
        self.cache.retain(|_, entry| entry.strong_count() != 0);
        changed
    }
}

fn read_limited(reader: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_ENCODED_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_ENCODED_BYTES {
        return Err("image exceeds 16 MiB encoded limit".into());
    }
    Ok(bytes)
}

fn load(agent: &ureq::Agent, source: &ImageSource) -> Result<Vec<u8>, String> {
    match source {
        ImageSource::Embedded { bytes, .. } => read_limited(Cursor::new(bytes.as_ref())),
        ImageSource::Url(url) => {
            let url = url.trim();
            if url.starts_with("http://") || url.starts_with("https://") {
                let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
                read_limited(response.body_mut().as_reader())
            } else if url.contains("://") || url.starts_with("//") {
                Err("desktop images require HTTP(S), embedded bytes, or a filesystem path".into())
            } else {
                read_limited(std::fs::File::open(url).map_err(|e| e.to_string())?)
            }
        }
    }
}

fn decode(id: ImageId, bytes: &[u8]) -> Result<RasterImage, String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    use image::ImageDecoder;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) * 4 > MAX_DECODED_BYTES {
        return Err("decoded image exceeds 64 MiB limit".into());
    }
    let mut pixels = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    pixels.apply_orientation(orientation);
    let pixels = pixels.into_rgba8();
    Ok(RasterImage {
        id,
        width: pixels.width(),
        height: pixels.height(),
        rgba: pixels.into_raw().into(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use refract_core::ImageFormat;

    pub(crate) fn png_source() -> ImageSource {
        colored_png_source([80, 140, 210, 128])
    }

    fn colored_png_source(color: [u8; 4]) -> ImageSource {
        let pixels = image::RgbaImage::from_pixel(4, 2, image::Rgba(color));
        let mut encoded = Cursor::new(Vec::new());
        pixels
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        ImageSource::embedded(ImageFormat::Png, encoded.into_inner())
    }

    // Hold the receiver until every image is acquired, making saturation
    // deterministic regardless of how fast the machine decodes small PNGs.
    fn paused_store() -> (ImageStore, mpsc::Receiver<Job>, mpsc::Receiver<()>) {
        let (wake, wakes) = mpsc::channel();
        let mut store = ImageStore::new(move || {
            let _ = wake.send(());
        });
        let (jobs, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        store.jobs = Some(jobs);
        (store, receiver, wakes)
    }

    fn finish_images(store: &mut ImageStore, handles: &[ImageHandle], wakes: &mpsc::Receiver<()>) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while handles
            .iter()
            .any(|handle| matches!(handle.borrow().state, ImageState::Loading))
        {
            // Production waits for worker notifications too: no timer or unrelated
            // user input should be required to admit the rest of the gallery.
            wakes
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("image loading stopped making progress");
            store.poll();
        }
        for handle in handles {
            assert!(
                matches!(handle.borrow().state, ImageState::Ready(_)),
                "every valid image must load"
            );
        }
    }

    #[test]
    fn gallery_larger_than_worker_queue_eventually_loads_every_image() {
        let (mut store, receiver, wakes) = paused_store();
        let sources: Vec<_> = (0..QUEUE_CAPACITY * 3)
            .map(|index| colored_png_source([index as u8, 140, 210, 255]))
            .collect();
        let handles: Vec<_> = sources.iter().map(|source| store.acquire(source)).collect();
        let shared = store.acquire(sources.last().unwrap());
        assert!(Rc::ptr_eq(&shared, handles.last().unwrap()));
        assert!(
            handles
                .iter()
                .all(|handle| matches!(handle.borrow().state, ImageState::Loading)),
            "queue saturation must not fail a valid image"
        );

        store.spawn_workers(receiver);
        finish_images(&mut store, &handles, &wakes);
        assert_eq!(store.decoded, sources.len() as u64);
        assert!(store.pending.is_empty());
        assert!(store.deferred.is_empty());
    }

    #[test]
    fn unmounted_deferred_images_are_skipped_and_reacquired_sources_still_load() {
        let (mut store, receiver, wakes) = paused_store();
        let mut handles: Vec<_> = (0..QUEUE_CAPACITY)
            .map(|index| store.acquire(&colored_png_source([index as u8, 0, 0, 255])))
            .collect();
        let source = png_source();
        let unmounted = store.acquire(&source);
        let old_id = unmounted.borrow().id;
        let weak = Rc::downgrade(&unmounted);
        drop(unmounted);
        let replacement = store.acquire(&source);
        assert_ne!(replacement.borrow().id, old_id);
        assert!(weak.upgrade().is_none());
        handles.push(replacement);

        // Workers are still paused and the queue remains full. Retired requests
        // must be skipped before attempting to admit the replacement.
        assert!(store.poll().is_empty());
        assert!(!store.pending.contains_key(&old_id));
        assert_eq!(store.deferred.len(), 1);
        store.spawn_workers(receiver);
        finish_images(&mut store, &handles, &wakes);
        assert_eq!(store.decoded, handles.len() as u64);
        assert!(store.pending.is_empty());
        assert!(store.deferred.is_empty());
    }

    #[test]
    fn disconnected_workers_fail_deferred_images_instead_of_leaving_them_loading() {
        let (mut store, receiver, _) = paused_store();
        let handles: Vec<_> = (0..QUEUE_CAPACITY + 1)
            .map(|index| store.acquire(&colored_png_source([index as u8, 0, 0, 255])))
            .collect();
        let deferred_id = handles.last().unwrap().borrow().id;
        drop(receiver);
        assert_eq!(store.poll(), [deferred_id]);
        assert!(matches!(&handles.last().unwrap().borrow().state,
            ImageState::Failed(error) if error == "image workers disconnected"));
        assert!(store.deferred.is_empty());
        assert!(!store.pending.contains_key(&deferred_id));
    }

    fn wait(store: &mut ImageStore, handle: &ImageHandle) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            store.poll();
            if !matches!(handle.borrow().state, ImageState::Loading) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "image worker timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn sources_share_decoding_and_pixels_preserve_straight_alpha() {
        let mut store = ImageStore::new(|| {});
        let source = png_source();
        let first = store.acquire(&source);
        let second = store.acquire(&source);
        assert!(Rc::ptr_eq(&first, &second));
        wait(&mut store, &first);
        let resource = first.borrow();
        let ImageState::Ready(image) = &resource.state else {
            panic!("expected decoded pixels")
        };
        assert_eq!((image.width(), image.height()), (4, 2));
        assert_eq!(&image.rgba()[..4], &[80, 140, 210, 128]);
        assert_eq!(store.decoded, 1);
    }

    #[test]
    fn corrupt_images_fail_without_faulting_the_host() {
        let mut store = ImageStore::new(|| {});
        let handle = store.acquire(&ImageSource::embedded(
            ImageFormat::Png,
            b"not an image".as_slice(),
        ));
        wait(&mut store, &handle);
        assert!(
            matches!(&handle.borrow().state, ImageState::Failed(message) if !message.is_empty())
        );
        assert_eq!(store.decoded, 0);
    }

    #[test]
    fn released_resources_discard_late_completions() {
        let mut store = ImageStore::new(|| {});
        let handle = store.acquire(&png_source());
        let weak = Rc::downgrade(&handle);
        drop(handle);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !store.pending.is_empty() {
            assert!(store.poll().is_empty());
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(weak.upgrade().is_none());
        assert!(store.cache.is_empty());
        assert_eq!(store.decoded, 0);
    }

    #[test]
    fn http_sources_load_on_workers_without_ui_thread_network_io() {
        use std::io::Write;
        let source = png_source();
        let ImageSource::Embedded { bytes, .. } = source else {
            unreachable!()
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/photo.png", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            {
                let mut reader = std::io::BufReader::new(&mut stream);
                let mut request = String::new();
                while !request.ends_with("\r\n\r\n") {
                    assert!(std::io::BufRead::read_line(&mut reader, &mut request).unwrap() > 0);
                    assert!(request.len() < 8192);
                }
            }
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).unwrap();
            stream.write_all(&bytes).unwrap();
        });
        let mut store = ImageStore::new(|| {});
        let handle = store.acquire(&url.into());
        wait(&mut store, &handle);
        assert!(matches!(handle.borrow().state, ImageState::Ready(_)));
        server.join().unwrap();
    }
}
