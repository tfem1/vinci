//! End-to-end engine tests using the synthetic `Test` backend (real FFmpeg encoder,
//! real replay buffer, real export, real SQLite library).

use super::*;

fn ffmpeg_with_x264() -> bool {
    vinci_hw::ffmpeg::locate(None).map(|f| f.has_encoder("libx264") && f.has_input_device("lavfi") && f.ffprobe.is_some()).unwrap_or(false)
}

fn wait_for(mut cond: impl FnMut() -> bool, secs: u64) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn test_engine(dir: &Path, storage: BufferStorage) -> Engine {
    let paths = AppPaths::rooted(dir);
    let e = Engine::new(paths).unwrap();
    let mut s = e.settings();
    s.capture.backend = BackendKind::Test;
    s.capture.resolution = vinci_capture::Resolution::P720;
    s.capture.fps = 30;
    s.encoding.codec = CodecChoice::H264;
    s.encoding.encoder = Some("libx264".into());
    s.encoding.keyframe_interval_secs = 1.0;
    s.replay.buffer_secs = 15;
    s.replay.storage = storage;
    s.library.organize_by_game = true;
    e.update_settings(s).unwrap();
    e
}

#[test]
fn replay_save_library_and_compress() {
    if !ffmpeg_with_x264() {
        eprintln!("skipping: ffmpeg with libx264/lavfi/ffprobe not available");
        return;
    }
    for storage in [BufferStorage::Memory, BufferStorage::Disk] {
        let dir = tempfile::tempdir().unwrap();
        let e = test_engine(dir.path(), storage);
        let events = e.subscribe();
        e.start_replay().unwrap();
        assert!(wait_for(|| e.status().buffered_secs >= 7.0, 40), "buffer never filled: {:?}", e.status());
        let st = e.status();
        assert_eq!(st.state, ReplayState::Running);
        assert_eq!(st.buffer_storage, if storage == BufferStorage::Disk { "disk" } else { "memory" });
        assert_eq!(st.session.as_ref().unwrap().encoder, "libx264");

        let t0 = Instant::now();
        let clip = e.save_clip_blocking(Some(5)).expect("save");
        let save_ms = t0.elapsed().as_millis();
        assert!(clip.path.exists());
        // Clips start on a keyframe (≤ 2 s before the requested start) and end on a
        // 0.5 s fragment; audio can add a few ms on a slow runner.
        assert!(clip.duration >= 5.0 && clip.duration <= 8.0, "duration {}", clip.duration);
        assert_eq!((clip.width, clip.height), (1280, 720));
        assert_eq!(clip.audio_codec.as_deref(), Some("aac"));
        assert!(clip.thumbnail.as_ref().is_some_and(|t| t.exists()), "thumbnail generated");
        assert!(save_ms < 5000, "save took {save_ms} ms");
        assert!(events.try_iter().any(|ev| matches!(ev, EngineEvent::ClipSaved { .. })));
        assert_eq!(e.list_clips(&ClipQuery::default()).unwrap().len(), 1);

        // Asking for more than is buffered returns everything that is buffered.
        let all = e.save_clip_blocking(Some(300)).unwrap();
        assert!(all.duration >= 7.0, "{}", all.duration);

        e.stop_replay();
        assert_eq!(e.status().state, ReplayState::Stopped);

        // Compress in place to ≤ 1 MB: still one physical file.
        let job = e.start_job(
            clip.id,
            JobKind::Compress {
                options: CompressOptions {
                    codec: vinci_encoding::Codec::H264,
                    target: vinci_encoding::CompressTarget::SizeMb(1.0),
                    max_height: None,
                    max_fps: None,
                },
                replace: true,
            },
        );
        let mut done = None;
        assert!(wait_for(
            || {
                for ev in events.try_iter() {
                    if let EngineEvent::JobDone { job_id, clip, error, .. } = ev {
                        if job_id == job {
                            done = Some((clip, error));
                        }
                    }
                }
                done.is_some()
            },
            60
        ));
        let (c, err) = done.unwrap();
        assert!(err.is_none(), "{err:?}");
        let c = c.unwrap();
        assert_eq!(c.id, clip.id);
        assert!(c.size <= 1_000_000, "{}", c.size);
        assert_eq!(e.list_clips(&ClipQuery::default()).unwrap().len(), 2, "no duplicate created");
        let files: Vec<_> = std::fs::read_dir(clip.path.parent().unwrap()).unwrap().flatten().map(|f| f.file_name()).collect();
        assert_eq!(files.len(), 2, "exactly two clip files on disk: {files:?}");

        e.shutdown();
        if storage == BufferStorage::Disk {
            let rings: Vec<_> = std::fs::read_dir(AppPaths::rooted(dir.path()).ring_dir).unwrap().collect();
            assert!(rings.is_empty(), "disk ring removed on shutdown");
        }
    }
}

#[test]
fn live_buffer_length_change_keeps_footage() {
    if !ffmpeg_with_x264() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let e = test_engine(dir.path(), BufferStorage::Memory);
    e.start_replay().unwrap();
    assert!(wait_for(|| e.status().buffered_secs >= 3.0, 30));
    let session_before = e.status().since_ms;
    let mut s = e.settings();
    s.replay.buffer_secs = 30;
    e.update_settings(s).unwrap();
    assert_eq!(e.status().since_ms, session_before, "pipeline not restarted");
    assert!(e.status().buffered_secs >= 3.0);
    e.shutdown();
}

#[test]
fn save_without_buffer_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let e = Engine::new(AppPaths::rooted(dir.path())).unwrap();
    let err = e.save_clip_blocking(Some(30)).unwrap_err();
    assert!(err.contains("not recording") && err.contains("Start recording"), "{err}");
    e.shutdown();
}

#[test]
fn estimate_reports_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let e = Engine::new(AppPaths::rooted(dir.path())).unwrap();
    let mut s = Settings::default();
    s.replay.buffer_secs = 300;
    let est = e.estimate(&s);
    assert!(est.mb_per_minute > 10.0 && est.buffer_mb > est.mb_per_minute);
    assert_eq!(est.clip_mb.len(), 5);
    e.shutdown();
}
