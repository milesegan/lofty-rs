//! Items for OGG container formats
//!
//! ## File notes
//!
//! The only supported tag format is [`VorbisComments`]
pub(crate) mod constants;
pub(crate) mod opus;
mod picture_storage;
pub(crate) mod read;
pub(crate) mod speex;
pub(crate) mod tag;
pub(crate) mod vorbis;
pub(crate) mod write;

use crate::error::Result;
use crate::macros::decode_err;

use std::io::{Read, Seek, SeekFrom};

use ogg_pager::{Page, PageHeader};

// Exports

pub use opus::OpusFile;
pub use opus::properties::OpusProperties;
pub use picture_storage::OggPictureStorage;
pub use speex::SpeexFile;
pub use speex::properties::SpeexProperties;
pub use tag::VorbisComments;
pub use vorbis::VorbisFile;
pub use vorbis::properties::VorbisProperties;

fn verify_signature(content: &[u8], sig: &[u8]) -> Result<()> {
	let sig_len = sig.len();

	if content.len() < sig_len || &content[..sig_len] != sig {
		decode_err!(@BAIL Vorbis, "File missing magic signature");
	}

	Ok(())
}


// Accessing the private `crc32` from `ogg_pager` via `pub use` in `lib.rs` of `ogg_pager`?
// The file `lofty/src/ogg/mod.rs` uses `ogg_pager`.
// I checked `ogg_pager/src/lib.rs` and it has `pub use crc::crc32;`.
// So `ogg_pager::crc32` is available.

fn find_last_page<R>(data: &mut R) -> Result<PageHeader>
where
	R: Read + Seek,
{
	let start_pos = data.stream_position()?;
	let file_len = data.seek(SeekFrom::End(0))?;

	// 64KB chunk size
	// 8KB chunk size
	const CHUNK_SIZE: u64 = 8192;

	let mut buffer = vec![0; CHUNK_SIZE as usize];
	let mut current_pos = file_len;

	while current_pos > start_pos {
		let size = std::cmp::min(CHUNK_SIZE, current_pos - start_pos);
		let search_start = current_pos - size;

		data.seek(SeekFrom::Start(search_start))?;
		data.read_exact(&mut buffer[..size as usize])?;

		let chunk = &buffer[..size as usize];

		// Scan backwards in the chunk
		for i in (0..chunk.len()).rev() {
			// Check for "OggS" (capture pattern) ending at i
			// "OggS" is 4 bytes. If buffer[i] is 'S' (index 3 of pattern),
			// then pattern starts at i-3.
			if chunk[i] == b'S' {
				if i >= 3 && &chunk[i - 3..i] == b"Ogg" {
					let header_start = search_start + (i - 3) as u64;

					data.seek(SeekFrom::Start(header_start))?;

					// Try to read header first
					let header = match PageHeader::read(data) {
						Ok(h) => h,
						Err(_) => continue, // False positive or partial overwrite
					};

					// Calculate expected end
					let header_len = data.stream_position()? - header_start;
					let content_len = header.content_size() as u64;
					let page_end = header_start + header_len + content_len;

					// Optimization: If the page ends exactly at the end of the file, we can skip the CRC check
					// We also allow for a small amount of trailing garbage (e.g. up to 128 bytes for ID3v1, though unlikely in Ogg)
					// But strict check is safer for skipping CRC. Use strict check.
					if page_end == file_len {
						return Ok(header);
					}

					// Fallback: Verify CRC
					// Seek back and read full page
					data.seek(SeekFrom::Start(header_start))?;
					if let Ok(mut page) = Page::read(data) {
						// Check CRC (using public getter)
						let orig = page.header().checksum();
						page.gen_crc();

						if page.header().checksum() == orig {
							return Ok(page.header().clone());
						}
					}
				}
			}
		}

		// Overlap by 3 bytes to catch "OggS" crossing chunk boundaries
		if search_start < 3 {
			break;
		}
		current_pos = search_start + 3;
	}

	// Fallback to forward scan if backward scan failure
	// (e.g. file too small to have a valid page, or corruption)
	data.seek(SeekFrom::Start(start_pos))?;

	let mut last_page_header = PageHeader::read(data)?;
	data.seek(SeekFrom::Current(last_page_header.content_size() as i64))?;

	while let Ok(header) = PageHeader::read(data) {
		last_page_header = header;
		data.seek(SeekFrom::Current(last_page_header.content_size() as i64))?;
	}

	Ok(last_page_header)
}
