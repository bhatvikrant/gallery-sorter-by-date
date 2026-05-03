//! Bucket extracted metadata into per-day groups, sorted oldest-first.
//! Mirrors `src/grouper.ts`.

use crate::date_format::{date_key, format_for_display, format_for_folder, parse_date_key};
use crate::types::{DateGroup, MediaMeta, MediaType};
use std::collections::HashMap;

pub struct GroupResult {
    pub groups: Vec<DateGroup>,
    pub total_images: usize,
    pub total_videos: usize,
}

pub fn group_by_date(files: Vec<MediaMeta>) -> GroupResult {
    let mut bucket: HashMap<String, Vec<MediaMeta>> = HashMap::new();
    for f in files {
        bucket
            .entry(date_key(&f.created_date))
            .or_default()
            .push(f);
    }

    let mut groups: Vec<DateGroup> = Vec::with_capacity(bucket.len());
    let mut total_images = 0usize;
    let mut total_videos = 0usize;

    for (key, files) in bucket {
        let date = parse_date_key(&key).expect("internally generated key always parses");
        let mut image_count = 0usize;
        let mut video_count = 0usize;
        for f in &files {
            match f.file.media_type {
                MediaType::Image => image_count += 1,
                MediaType::Video => video_count += 1,
            }
        }
        total_images += image_count;
        total_videos += video_count;
        groups.push(DateGroup {
            date_key: key,
            folder_path: format_for_folder(&date),
            display_name: format_for_display(&date),
            date,
            files,
            image_count,
            video_count,
        });
    }

    groups.sort_by_key(|g| g.date.timestamp());

    GroupResult {
        groups,
        total_images,
        total_videos,
    }
}
