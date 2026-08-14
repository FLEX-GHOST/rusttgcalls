//! Runnable example wiring rusttgcalls against a Telegram MTProto client.
//!
//! Flow:
//!  1. MTProto client fetches the active group-call reference.
//!  2. rusttgcalls.create_call produces local-side JSON.
//!  3. Telegram's phone.joinGroupCall sends that JSON and returns Telegram's response JSON.
//!  4. rusttgcalls.connect feeds that response back and finishes the WebRTC handshake.
//!  5. rusttgcalls.set_stream_sources starts the streamer.
//!  6. On SIGINT, we leave the call cleanly via phone.leaveGroupCall.

use rusttgcalls::{
    Client, EncodeOptions, TRACK_AUDIO, TRACK_VIDEO, from_file, from_shell, from_shells, from_url,
};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let source_arg = if args.len() > 1 { &args[1] } else { "song.mp3" };

    // Initialize rusttgcalls client
    let client = Client::new()?;
    let chat_id: i64 = -1001234567890;

    // Stream lifecycle events. Fires on natural EOF (err == None) or ffmpeg crash.
    client
        .on_stream_end(|chat, stream_type, device, err| {
            println!(
                "Stream end chat={} type={:?} device={:?} err={:?}",
                chat, stream_type, device, err
            );
        })
        .await;

    // ICE/DTLS connection state transitions.
    client
        .on_connection_change(|chat, info| {
            println!("Connection change chat={} state={:?}", chat, info.state);
        })
        .await;

    // Fires on Mute/Unmute/Pause/Resume and spontaneous transitions.
    client
        .on_upgrade(|chat, state| {
            println!(
                "Upgrade chat={} muted={} paused={} video_stopped={}",
                chat, state.muted, state.paused, state.video_stopped
            );
        })
        .await;

    // 1. Generate local-side JSON parameters (for phone.joinGroupCall)
    let local_params = client.create_call(chat_id).await?;
    println!("Step 1: Generated Local JSON Params ({} bytes):\n{}", local_params.len(), local_params);

    // 2. Drive Telegram via your MTProto layer:
    //    Pass `local_params` string to `phone.joinGroupCall` with `video_stopped: !is_video`.
    //    (Set video_stopped: true for audio-only to prevent empty video window on mobile, false for video).
    //    Example MTProto skeleton:
    //    ```rust
    //    let updates = grammers_or_other_client.invoke(&functions::phone::JoinGroupCall {
    //        call: active_input_group_call,
    //        params: types::DataJson { data: local_params },
    //        video_stopped: !is_video, // true for audio-only, false for video
    //        ...
    //    }).await?;
    //    let remote_params = extract_json_blob_from_updates(&updates);
    //    // Note: For mid-call video toggles in active calls, use phone::EditGroupCallParticipant.
    //    ```

    // 3. Finish WebRTC handshake:
    //    Feed the SFU's JSON response back to rusttgcalls to establish the connection:
    //    ```rust
    //    client.connect(chat_id, &remote_params).await?;
    //    ```

    // 4. Stream media source (from_file / from_url / from_shell / from_shells / from_url_offset)
    // You can choose 60 FPS ultra-smooth, 30 FPS standard, HD, or custom presets:
    // - EncodeOptions::audio_only()          -> Studio fullband Opus audio
    // - EncodeOptions::video_default()       -> 480p @ 60 FPS (Ultra-smooth real-time on any CPU)
    // - EncodeOptions::video_60fps()         -> Alias for 60 FPS
    // - EncodeOptions::video_720p_60fps()    -> 720p HD @ 60 FPS (High-end servers)
    // - EncodeOptions::video_1080p_60fps()   -> 1080p FHD @ 60 FPS
    // - EncodeOptions::video_720p_30fps()    -> 720p HD @ 30 FPS
    // - EncodeOptions::video_fast()          -> 360p @ 30 FPS (Low-latency lightweight)
    // Or customize any resolution and FPS manually:
    // let custom_opt = EncodeOptions {
    //     video_width: 1280,
    //     video_height: 720,
    //     video_fps: 60, // 60, 30, 24, etc.
    //     video_bitrate_kbps: 2000,
    //     ..Default::default()
    // };
    let opt = if source_arg.contains("60fps") {
        EncodeOptions::video_60fps()
    } else if source_arg.contains("720p") {
        EncodeOptions::video_720p_30fps()
    } else {
        EncodeOptions::video_default()
    };

    let _streams = if source_arg.starts_with("shell:") {
        from_shell(source_arg.trim_start_matches("shell:"), TRACK_AUDIO)?
    } else if source_arg.starts_with("shellv:") {
        from_shell(source_arg.trim_start_matches("shellv:"), TRACK_VIDEO)?
    } else if source_arg.starts_with("shells:") {
        let rest = source_arg.trim_start_matches("shells:");
        let parts: Vec<&str> = rest.split('|').collect();
        let audio_cmd = parts.first().copied().unwrap_or("");
        let video_cmd = parts.get(1).copied().unwrap_or("");
        from_shells(audio_cmd, video_cmd)?
    } else if source_arg.starts_with("http://")
        || source_arg.starts_with("https://")
        || source_arg.starts_with("rtmp://")
        || source_arg.starts_with("rtsp://")
    {
        from_url(source_arg, opt)?
    } else {
        from_file(source_arg, opt)?
    };

    println!("\nStep 4: Media sources prepared successfully (configured FPS: {}, Resolution: {}x{}).", opt.video_fps, opt.video_width, opt.video_height);
    println!("To start streaming in an active call, pass streams to set_stream_sources:");
    println!("  client.set_stream_sources(chat_id, streams).await?;");

    // Optional runtime controls while streaming:
    // client.pause(chat_id).await?;
    // client.resume(chat_id).await?;
    // client.mute(chat_id).await?;
    // client.unmute(chat_id).await?;
    // client.seek_by(chat_id, 30_000).await?;
    // client.seek_to(chat_id, 60_000).await?;

    println!("\nPress Ctrl+C to clean up and exit...");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\nReceived Ctrl+C, tearing down call cleanly...");
        }
    }

    // Step 5: Clean shutdown
    client.stop(chat_id).await?;
    println!("Call closed cleanly.");

    Ok(())
}
