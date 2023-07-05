//! combines track audio and art into an embeddable video

use anyhow::*;
use image::RgbImage;
use log::{debug, error};
use serde::Deserialize;
use std::{collections::VecDeque, io::Cursor};
use webm::mux::Track;

use crate::requests::{request_bytes, request_image, request_text};

// https://github.com/astraw/vpx-encode/blob/master/record-screen/src/convert.rs
fn rgb_to_i420(image: &RgbImage) -> Vec<u8> {
    fn clamp(x: i32) -> u8 {
        x.min(255).max(0) as u8
    }

    let mut dest = Vec::new();

    for y in 0..image.height() {
        for x in 0..image.width() {
            let (r, g, b) = image.get_pixel(x, y).0.into();
            let y = (66 * r as i32 + 129 * g as i32 + 25 * b as i32 + 128) / 256 + 16;
            dest.push(clamp(y));
        }
    }

    for y in (0..image.height()).step_by(2) {
        for x in (0..image.width()).step_by(2) {
            let (r, g, b) = image.get_pixel(x, y).0.into();
            let u = (-38 * r as i32 - 74 * g as i32 + 112 * b as i32 + 128) / 256 + 128;
            dest.push(clamp(u));
        }
    }

    for y in (0..image.height()).step_by(2) {
        for x in (0..image.width()).step_by(2) {
            let (r, g, b) = image.get_pixel(x, y).0.into();
            let v = (112 * r as i32 - 94 * g as i32 - 18 * b as i32 + 128) / 256 + 128;
            dest.push(clamp(v));
        }
    }

    dest
}

/// encodes a video from the given hls stream and art. this takes a long time due to having to download a lot of data!
pub async fn encode_video(hls_url: &str, art_url: &str) -> Result<Vec<u8>> {
    #[derive(Deserialize)]
    struct UrlResult {
        url: String,
    }

    let res: UrlResult = serde_json::from_str(&request_text(hls_url).await?)?;

    let playlist = request_text(&res.url).await?;

    let urls = playlist.split('\n').filter(|line| !line.starts_with('#')).map(|line| line.to_string()).collect::<VecDeque<_>>();

    // spawn a task to download all the audio from the hls stream
    let download_task = tokio::spawn(async {
        let mut data = Vec::new();

        for url in urls {
            debug!("downloading audio from {url}");
            data.append(&mut request_bytes(&url).await?);
        }

        Ok(data)
    });

    let mut out = Vec::new();
    {
        let mut webm = webm::mux::Segment::new(webm::mux::Writer::new(Cursor::new(&mut out))).context("couldn't create new segment")?;

        // encode the cover art into a vp8 frame. this is done first because of how horrendously long it takes to download the audio
        let image_bytes = request_image(art_url).await?;
        let cover_art = image::io::Reader::with_format(Cursor::new(image_bytes), image::ImageFormat::Jpeg).decode()?.to_rgb8();

        let mut vt = webm.add_video_track(cover_art.width(), cover_art.height(), Some(1), webm::mux::VideoCodecId::VP8);
        // this segfaults if done earlier lmao
        if !vt.set_color(8, (true, true), false) {
            return Err(anyhow!("webm writer can't set color"));
        }

        // video frames have to be added after audio frames because otherwise things break, but they have to be encoded first because downloading takes ages
        struct Frame {
            data: Vec<u8>,
            key: bool,
            pts: i64,
        }

        let mut frames = Vec::with_capacity(1);

        {
            let mut vpx = vpx_encode::Encoder::new(vpx_encode::Config {
                width: cover_art.width(),
                height: cover_art.height(),
                timebase: [1, 1000],
                bitrate: 128,
                codec: vpx_encode::VideoCodecId::VP8,
            })
            .unwrap();

            let data = rgb_to_i420(&cover_art);
            for frame in vpx.encode(0, &data)? {
                frames.push(Frame {
                    data: frame.data.to_vec(),
                    key: frame.key,
                    pts: frame.pts,
                });
            }

            let mut new_frames = vpx.finish()?;
            while let Some(frame) = new_frames.next()? {
                frames.push(Frame {
                    data: frame.data.to_vec(),
                    key: frame.key,
                    pts: frame.pts,
                });
            }
        }

        // dump opus packets into the webm
        let sample_rate = 48000;
        let ns_per_sec = 100000000;
        let ns_per_sample = ns_per_sec / sample_rate;

        let mut at = webm.add_audio_track(sample_rate as i32, 2, None, webm::mux::AudioCodecId::Opus);
        let decoder = opus::Decoder::new(sample_rate as u32, opus::Channels::Stereo)?;

        let mut offset = 0;

        let mut cursor = Cursor::new(download_task.await??);
        let mut reader = ogg::PacketReader::new(&mut cursor);

        while let Some(packet) = reader.read_packet()? {
            match decoder.get_nb_samples(&packet.data) {
                Result::Ok(samples) => {
                    if !at.add_frame(&packet.data, offset, false) {
                        return Err(anyhow!("couldn't add audio frame"));
                    }
                    offset += (samples as u64) * ns_per_sample;
                }
                Err(err) => error!("couldn't parse packet: {err}"),
            }
        }

        for frame in frames {
            debug!("adding {}b frame @ {} (key {})", frame.data.len(), frame.pts, frame.key);
            if !vt.add_frame(&frame.data, frame.pts as u64 * 1000000, frame.key) {
                return Err(anyhow!("couldn't add video frame"));
            }
        }

        if !webm.finalize(Some(offset / 100000)) {
            return Err(anyhow!("couldn't finalize webm"));
        }
    }

    Ok(out)
}
