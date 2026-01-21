//! Chunked extended message support.
//!
//! USB PD 3.0+ supports extended messages that can exceed the maximum packet size.
//! These messages are split into chunks of up to 26 bytes each.
//!
//! See USB PD Spec R3.2 Section 6.13.

use heapless::Vec;

use super::ExtendedHeader;
// Re-export for convenience
pub use super::ExtendedHeader as ChunkExtendedHeader;
use crate::protocol_layer::message::ParseError;
use crate::protocol_layer::message::header::{ExtendedMessageType, Header};

/// Maximum data bytes in a single extended message chunk.
pub const MAX_EXTENDED_MSG_CHUNK_LEN: usize = 26;

/// Maximum total extended message length (data only, excluding headers).
pub const MAX_EXTENDED_MSG_LEN: usize = 260;

/// Maximum number of chunks (260 / 26 = 10).
pub const MAX_CHUNKS: usize = MAX_EXTENDED_MSG_LEN / MAX_EXTENDED_MSG_CHUNK_LEN;

/// Information about a received chunk.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ChunkInfo {
    /// The chunk number (0-15).
    pub chunk_number: u8,
    /// Total data size from extended header.
    pub total_data_size: u16,
    /// Whether this is a request for the next chunk.
    pub request_chunk: bool,
    /// The message type.
    pub message_type: ExtendedMessageType,
    /// The message header (for building responses).
    pub header: Header,
}

/// Result of processing a chunked message.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ChunkResult<T> {
    /// Message is complete and fully assembled.
    Complete(T),
    /// Need more chunks. Contains the chunk number to request next.
    NeedMoreChunks(u8),
    /// Received a chunk request from the other side.
    ChunkRequested(u8),
}

/// Assembler for chunked extended messages.
///
/// This struct accumulates chunks and reassembles the complete message.
///
/// # Example
/// ```
/// use usbpd::protocol_layer::message::extended::chunked::{
///     ChunkedMessageAssembler, ChunkResult, MAX_EXTENDED_MSG_CHUNK_LEN,
/// };
/// use usbpd::protocol_layer::message::extended::ExtendedHeader;
/// use usbpd::protocol_layer::message::header::Header;
///
/// let mut assembler = ChunkedMessageAssembler::new();
///
/// // Simulate receiving a 30-byte message split into 2 chunks (26 + 4 bytes)
/// let full_data: [u8; 30] = core::array::from_fn(|i| i as u8);
///
/// // Process chunk 0 (first 26 bytes)
/// let header = Header(0x9191); // Extended message header
/// let ext_header = ExtendedHeader::new(30).with_chunked(true).with_chunk_number(0);
/// match assembler.process_chunk(header, ext_header, &full_data[..26]).unwrap() {
///     ChunkResult::NeedMoreChunks(next) => assert_eq!(next, 1),
///     _ => panic!("Expected NeedMoreChunks"),
/// }
///
/// // Process chunk 1 (remaining 4 bytes)
/// let ext_header = ExtendedHeader::new(30).with_chunked(true).with_chunk_number(1);
/// match assembler.process_chunk(header, ext_header, &full_data[26..]).unwrap() {
///     ChunkResult::Complete(data) => assert_eq!(&data[..], &full_data),
///     _ => panic!("Expected Complete"),
/// }
/// ```
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ChunkedMessageAssembler {
    /// Accumulated data buffer.
    buffer: Vec<u8, MAX_EXTENDED_MSG_LEN>,
    /// Expected total data size.
    expected_size: u16,
    /// Number of bytes received so far.
    received_bytes: usize,
    /// The message type being assembled.
    message_type: Option<ExtendedMessageType>,
    /// The original header template.
    header_template: Option<Header>,
    /// Next expected chunk number.
    next_chunk: u8,
    /// Whether assembly is in progress.
    in_progress: bool,
}

impl Default for ChunkedMessageAssembler {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkedMessageAssembler {
    /// Create a new chunked message assembler.
    pub const fn new() -> Self {
        Self {
            buffer: Vec::new(),
            expected_size: 0,
            received_bytes: 0,
            message_type: None,
            header_template: None,
            next_chunk: 0,
            in_progress: false,
        }
    }

    /// Reset the assembler state by creating a fresh instance.
    ///
    /// This ensures reset() and new() always stay in sync.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Create a new assembler and initialize it with chunk 0.
    ///
    /// This is a convenience method that combines `new()` and `process_chunk()` for the first chunk.
    ///
    /// # Arguments
    /// * `header` - The PD message header for chunk 0
    /// * `ext_header` - The extended message header for chunk 0
    /// * `chunk_data` - The chunk 0 payload data (without headers)
    ///
    /// # Returns
    /// * `Ok((assembler, result))` - New assembler and the result of processing chunk 0
    /// * `Err(ParseError)` - If chunk 0 is invalid (e.g., wrong chunk number)
    ///
    /// # Example
    /// ```ignore
    /// let (mut assembler, result) = ChunkedMessageAssembler::new_from_chunk(
    ///     header, ext_header, chunk_0_data
    /// )?;
    /// match result {
    ///     ChunkResult::Complete(data) => { /* Single chunk message */ },
    ///     ChunkResult::NeedMoreChunks(_) => { /* Continue with process_chunk() */ },
    ///     _ => unreachable!(),
    /// }
    /// ```
    pub fn new_from_chunk(
        header: Header,
        ext_header: ExtendedHeader,
        chunk_data: &[u8],
    ) -> Result<(Self, ChunkResult<Vec<u8, MAX_EXTENDED_MSG_LEN>>), ParseError> {
        let mut assembler = Self::new();
        let result = assembler.process_chunk(header, ext_header, chunk_data)?;
        Ok((assembler, result))
    }

    /// Check if assembly is currently in progress.
    pub fn is_in_progress(&self) -> bool {
        self.in_progress
    }

    /// Get the message type being assembled.
    pub fn message_type(&self) -> Option<ExtendedMessageType> {
        self.message_type
    }

    /// Process a received chunk.
    ///
    /// # Arguments
    /// * `header` - The PD message header
    /// * `ext_header` - The extended message header
    /// * `chunk_data` - The chunk payload data (without headers)
    ///
    /// # Returns
    /// * `ChunkResult::Complete` - All chunks received, returns assembled data
    /// * `ChunkResult::NeedMoreChunks` - Need to request more chunks
    /// * `ChunkResult::ChunkRequested` - This is a chunk request from peer
    pub fn process_chunk(
        &mut self,
        header: Header,
        ext_header: ExtendedHeader,
        chunk_data: &[u8],
    ) -> Result<ChunkResult<Vec<u8, MAX_EXTENDED_MSG_LEN>>, ParseError> {
        let chunk_number = ext_header.chunk_number();
        let data_size = ext_header.data_size();
        let request_chunk = ext_header.request_chunk();

        // If this is a chunk request, not actual data
        if request_chunk {
            return Ok(ChunkResult::ChunkRequested(chunk_number));
        }

        // Validate chunk number
        if chunk_number == 0 {
            // First chunk - ensure parser is not already in use
            if self.in_progress {
                return Err(ParseError::ParserReuse);
            }
            // Initialize assembler for new message
            self.expected_size = data_size;
            self.message_type = Some(header.message_type_raw().into());
            self.header_template = Some(header);
            self.in_progress = true;
            self.next_chunk = 0;
        } else if !self.in_progress {
            return Err(ParseError::Other("Received non-zero chunk without chunk 0"));
        } else if chunk_number != self.next_chunk {
            return Err(ParseError::Other("Unexpected chunk number"));
        }

        // Validate chunk size (should never exceed 26 bytes per spec)
        if chunk_data.len() > MAX_EXTENDED_MSG_CHUNK_LEN {
            return Err(ParseError::ChunkOverflow(chunk_data.len(), MAX_EXTENDED_MSG_CHUNK_LEN));
        }

        // Copy chunk data to buffer
        if self.buffer.extend_from_slice(chunk_data).is_err() {
            return Err(ParseError::Other("Chunk buffer overflow"));
        }
        self.received_bytes += chunk_data.len();
        self.next_chunk = chunk_number + 1;

        // Check if we have all the data
        if self.received_bytes >= self.expected_size as usize {
            self.in_progress = false;
            // Truncate to expected size if we received extra padding
            let final_size = core::cmp::min(self.buffer.len(), self.expected_size as usize);
            self.buffer.truncate(final_size);
            Ok(ChunkResult::Complete(self.buffer.clone()))
        } else {
            Ok(ChunkResult::NeedMoreChunks(self.next_chunk))
        }
    }

    /// Build a chunk request extended header.
    ///
    /// # Arguments
    /// * `chunk_number` - The chunk number to request
    ///
    /// # Returns
    /// Extended header configured for chunk request
    ///
    /// # Note
    /// The caller is responsible for building the full message header with
    /// the correct message ID, roles, and extended message type.
    pub fn build_chunk_request_header(chunk_number: u8) -> ExtendedHeader {
        ExtendedHeader::new(0)
            .with_chunked(true)
            .with_request_chunk(true)
            .with_chunk_number(chunk_number)
    }

    /// Get the assembled data buffer (for partial inspection).
    pub fn buffer(&self) -> &[u8] {
        &self.buffer
    }

    /// Get the number of bytes received so far.
    pub fn received_bytes(&self) -> usize {
        self.received_bytes
    }

    /// Get the expected total size.
    pub fn expected_size(&self) -> u16 {
        self.expected_size
    }
}

/// Helper to split data into chunks for sending.
pub struct ChunkedMessageSender<'a> {
    data: &'a [u8],
    current_chunk: u8,
    total_chunks: u8,
}

impl<'a> ChunkedMessageSender<'a> {
    /// Create a new chunked message sender.
    ///
    /// # Arguments
    /// * `data` - The complete message data to send
    pub fn new(data: &'a [u8]) -> Self {
        let total_chunks = if data.is_empty() {
            1
        } else {
            data.len().div_ceil(MAX_EXTENDED_MSG_CHUNK_LEN) as u8
        };

        Self {
            data,
            current_chunk: 0,
            total_chunks,
        }
    }

    /// Check if all chunks have been sent.
    pub fn is_complete(&self) -> bool {
        self.current_chunk >= self.total_chunks
    }

    /// Get the current chunk number.
    pub fn current_chunk(&self) -> u8 {
        self.current_chunk
    }

    /// Get the total number of chunks.
    pub fn total_chunks(&self) -> u8 {
        self.total_chunks
    }

    /// Get the total data size.
    pub fn data_size(&self) -> u16 {
        self.data.len() as u16
    }

    /// Get a specific chunk by number (for responding to chunk requests).
    pub fn get_chunk(&self, chunk_number: u8) -> Option<(ExtendedHeader, &[u8])> {
        if chunk_number >= self.total_chunks {
            return None;
        }

        let start = chunk_number as usize * MAX_EXTENDED_MSG_CHUNK_LEN;
        let end = core::cmp::min(start + MAX_EXTENDED_MSG_CHUNK_LEN, self.data.len());
        let chunk_data = &self.data[start..end];

        let ext_header = ExtendedHeader::new(self.data.len() as u16)
            .with_chunked(true)
            .with_chunk_number(chunk_number);

        Some((ext_header, chunk_data))
    }

    /// Reset to send from the beginning.
    pub fn reset(&mut self) {
        self.current_chunk = 0;
    }
}

impl<'a> Iterator for ChunkedMessageSender<'a> {
    type Item = (ExtendedHeader, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        if self.is_complete() {
            return None;
        }

        let start = self.current_chunk as usize * MAX_EXTENDED_MSG_CHUNK_LEN;
        let end = core::cmp::min(start + MAX_EXTENDED_MSG_CHUNK_LEN, self.data.len());
        let chunk_data = &self.data[start..end];

        let ext_header = ExtendedHeader::new(self.data.len() as u16)
            .with_chunked(true)
            .with_chunk_number(self.current_chunk);

        self.current_chunk += 1;

        Some((ext_header, chunk_data))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.total_chunks - self.current_chunk) as usize;
        (remaining, Some(remaining))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunked_sender_single_chunk() {
        let data = [1u8, 2, 3, 4, 5];
        let mut sender = ChunkedMessageSender::new(&data);

        assert_eq!(sender.total_chunks(), 1);
        assert!(!sender.is_complete());

        let (ext_hdr, chunk) = sender.next().unwrap();
        assert_eq!(chunk, &data);
        assert_eq!(ext_hdr.data_size(), 5);
        assert_eq!(ext_hdr.chunk_number(), 0);
        assert!(ext_hdr.chunked());

        assert!(sender.is_complete());
        assert!(sender.next().is_none());
    }

    #[test]
    fn test_chunked_sender_multiple_chunks() {
        // 30 bytes = 2 chunks (26 + 4)
        let data = [0u8; 30];
        let mut sender = ChunkedMessageSender::new(&data);

        assert_eq!(sender.total_chunks(), 2);

        let (ext_hdr, chunk) = sender.next().unwrap();
        assert_eq!(chunk.len(), 26);
        assert_eq!(ext_hdr.chunk_number(), 0);

        let (ext_hdr, chunk) = sender.next().unwrap();
        assert_eq!(chunk.len(), 4);
        assert_eq!(ext_hdr.chunk_number(), 1);

        assert!(sender.is_complete());
    }

    #[test]
    fn test_assembler_single_chunk() {
        let mut assembler = ChunkedMessageAssembler::new();

        let header = Header(0x1000); // Some header with extended bit
        let ext_header = ExtendedHeader::new(5).with_chunked(true).with_chunk_number(0);
        let data = [1u8, 2, 3, 4, 5];

        match assembler.process_chunk(header, ext_header, &data).unwrap() {
            ChunkResult::Complete(buf) => {
                assert_eq!(&buf[..], &data);
            }
            _ => panic!("Expected complete"),
        }
    }

    #[test]
    fn test_assembler_parser_reuse_error() {
        let mut assembler = ChunkedMessageAssembler::new();

        let header = Header(0x1000);
        let ext_header = ExtendedHeader::new(30).with_chunked(true).with_chunk_number(0);
        let data = [1u8; 26];

        // Process first chunk - should succeed
        match assembler.process_chunk(header, ext_header, &data).unwrap() {
            ChunkResult::NeedMoreChunks(next) => assert_eq!(next, 1),
            _ => panic!("Expected NeedMoreChunks"),
        }

        // Try to start a new message while previous one is in progress - should fail
        let result = assembler.process_chunk(header, ext_header, &data);
        assert!(matches!(result, Err(ParseError::ParserReuse)));
    }

    #[test]
    fn test_new_from_chunk() {
        let header = Header(0x1000);
        let ext_header = ExtendedHeader::new(5).with_chunked(true).with_chunk_number(0);
        let data = [1u8, 2, 3, 4, 5];

        // Create assembler from chunk 0
        let (assembler, result) = ChunkedMessageAssembler::new_from_chunk(header, ext_header, &data).unwrap();

        // Single chunk message should be complete immediately
        match result {
            ChunkResult::Complete(buf) => assert_eq!(&buf[..], &data),
            _ => panic!("Expected Complete"),
        }

        // Assembler should not be in progress after complete message
        assert!(!assembler.is_in_progress());
    }

    #[test]
    fn test_new_from_chunk_multi_chunk() {
        let header = Header(0x1000);
        let ext_header = ExtendedHeader::new(30).with_chunked(true).with_chunk_number(0);
        let chunk_0 = [0u8; 26];

        // Create assembler from chunk 0
        let (mut assembler, result) = ChunkedMessageAssembler::new_from_chunk(header, ext_header, &chunk_0).unwrap();

        // Multi-chunk message should need more chunks
        match result {
            ChunkResult::NeedMoreChunks(next) => assert_eq!(next, 1),
            _ => panic!("Expected NeedMoreChunks"),
        }

        // Assembler should be in progress
        assert!(assembler.is_in_progress());

        // Process chunk 1
        let ext_header_1 = ExtendedHeader::new(30).with_chunked(true).with_chunk_number(1);
        let chunk_1 = [0u8; 4];
        match assembler.process_chunk(header, ext_header_1, &chunk_1).unwrap() {
            ChunkResult::Complete(_) => {}
            _ => panic!("Expected Complete"),
        }

        // Now assembler should not be in progress
        assert!(!assembler.is_in_progress());
    }

    #[test]
    fn test_chunk_overflow_error() {
        let mut assembler = ChunkedMessageAssembler::new();

        let header = Header(0x1000);
        let ext_header = ExtendedHeader::new(30).with_chunked(true).with_chunk_number(0);
        // Create chunk larger than MAX_EXTENDED_MSG_CHUNK_LEN (26 bytes)
        let oversized_chunk = [0u8; 27];

        // Should return ChunkOverflow error
        let result = assembler.process_chunk(header, ext_header, &oversized_chunk);
        assert!(matches!(
            result,
            Err(ParseError::ChunkOverflow(27, MAX_EXTENDED_MSG_CHUNK_LEN))
        ));
    }

    #[test]
    fn test_chunked_sender_as_iterator() {
        // 30 bytes = 2 chunks (26 + 4)
        let data = [0u8; 30];
        let mut sender = ChunkedMessageSender::new(&data);

        // Use iterator to get chunks
        let (ext_hdr0, chunk0) = sender.next().unwrap();
        assert_eq!(ext_hdr0.chunk_number(), 0);
        assert_eq!(chunk0.len(), 26);

        let (ext_hdr1, chunk1) = sender.next().unwrap();
        assert_eq!(ext_hdr1.chunk_number(), 1);
        assert_eq!(chunk1.len(), 4);

        assert!(sender.next().is_none());
    }

    #[test]
    fn test_chunked_sender_for_loop() {
        let data = [1u8, 2, 3, 4, 5];
        let sender = ChunkedMessageSender::new(&data);

        let mut count = 0;
        for (ext_hdr, chunk) in sender {
            assert_eq!(ext_hdr.chunk_number(), count);
            assert_eq!(chunk, &data);
            count += 1;
        }
        assert_eq!(count, 1);
    }
}
