// Copyright 2021 Florian Jung <flo@windfis.ch>
//
// Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
// associated documentation files (the "Software"), to deal in the Software without restriction,
// including without limitation the rights to use, copy, modify, merge, publish, distribute,
// sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all copies or
// substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT
// NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
// NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
// DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

#![no_std]

#[cfg(test)]
extern crate std;

pub struct FlashAccessError();

pub trait FlashTrait {
	/// The flash size in bytes. Must be a multiple of `PAGE_SIZE`.
	const SIZE: usize;

	/// The granularity in bytes with which pages can be erased
	const PAGE_SIZE: usize;

	/// The granularity in bytes with which data can be written. A value of 3 is unsupported.
	const WORD_SIZE: usize;

	/// Value of the first byte of each erased word. usually 0xFF
	const ERASED_VALUE: u8;

	/// Erases the page starting at `address`. `address` must be a multiple of `PAGE_SIZE`
	fn erase_page(&mut self, address: usize) -> Result<(), FlashAccessError>;

	/// Writes `data` to `address`. `address` must be a multiple of `WORD_SIZE`.
	/// If `data.len()` is not a multiple of `WORD_SIZE`, undefined padding is added.
	fn write(&mut self, address: usize, data: &[u8]) -> Result<(), FlashAccessError>;

	/// Reads `data.len()` bytes from `address`. neither `address` nor `data.len()` need to be multiples of `WORD_SIZE`. `address` is guaranteed to be a multiple of 4.
	fn read(&mut self, address: usize, data: &mut [u8]) -> Result<(), FlashAccessError>;
}

pub struct FlashStore<Flash: FlashTrait, const PAGE_SIZE: usize> {
	flash: Flash
}

#[derive(Debug, Clone, Copy, PartialEq)] // GRCOV_EXCL_LINE
pub enum FlashStoreError {
	NotFound,
	BufferTooSmall,
	CorruptData,
	NoSpaceLeft,
	FlashAccessError,
}

impl From<FlashAccessError> for FlashStoreError {
	fn from(_: FlashAccessError) -> FlashStoreError { FlashStoreError::FlashAccessError }
}

enum FindResult {
	/// holds the header offset and size of the file
	Found(usize, usize),

	/// holds the end-of-storage pointer
	NotFound(usize)
}

const HEADER_SIZE: usize = 4;

impl<Flash: FlashTrait, const PAGE_SIZE: usize> FlashStore<Flash, PAGE_SIZE> {
	pub fn new(flash: Flash) -> FlashStore<Flash, PAGE_SIZE> {
		assert!(Flash::WORD_SIZE != 3, "A word size of 3 is unsupported");
		assert!(Flash::PAGE_SIZE == PAGE_SIZE);
		FlashStore { flash }
	}
	
	fn parse_header(header: [u8; 4]) -> (u8, usize) {
		let number = (!header[0]) ^ Flash::ERASED_VALUE;
		let size = u32::from_le_bytes([header[1], header[2], header[3], 0]);
		(number, size as usize)
	}
	
	fn make_header(number: u8, size: usize) -> [u8; 4] {
		assert!(size <= 0xFFFFFF);
		let s = (size as u32).to_le_bytes();
		[(!number) ^ Flash::ERASED_VALUE, s[0], s[1], s[2]]
	}


	fn round(offset: usize) -> usize {
		let word_size = Flash::WORD_SIZE.max(HEADER_SIZE);
		((offset - 1) / word_size + 1) * word_size
	}

	fn read_header(&mut self, position: usize) -> Result<(u8, usize), FlashStoreError> {
		let mut header: [u8; 4] = [0,0,0,0];
		self.flash.read(position, &mut header)?;
		Ok(Self::parse_header(header))
	}

	fn find(&mut self, file_number: Option<u8>) -> Result<FindResult, FlashStoreError> {
		use FlashStoreError::*;

		let file_number = file_number.unwrap_or(0xFF);
		let mut position = 0;
		let mut found = None;

		while position < Flash::SIZE {
			let (number, size) = self.read_header(position)?;

			if number == 0xFF {
				// we read an uninitialized header, i.e. we hit end-of-storage
				break;
			}
			else if number == file_number {
				found = Some((position, size));
			}

			if position + HEADER_SIZE + size > Flash::SIZE {
				// the file exceeds the flash size
				return Err(CorruptData);
			}

			position += Self::round(HEADER_SIZE + size);
		}
		
		return match found {
			Some((found_at_offset, size)) => Ok(FindResult::Found(found_at_offset, size)),
			None => Ok(FindResult::NotFound(position))
		};
	}

	fn end_of_store(&mut self) -> Result<usize, FlashStoreError> {
		if let FindResult::NotFound(position) = self.find(None)? {
			return Ok(position);
		}
		else {
			unreachable!(); // GRCOV_EXCL_LINE
		}
	}

	/// file_number must not be 0xFF
	pub fn read_file<'a>(&mut self, file_number: u8, buffer: &'a mut [u8]) -> Result<&'a [u8], FlashStoreError> {
		assert!(file_number != 0xFF, "Illegal file number 0xFF");

		match self.find(Some(file_number))? {
			FindResult::Found(position, size) => {
				if buffer.len() < size {
					return Err(FlashStoreError::BufferTooSmall);
				}
				else {
					self.flash.read(position + HEADER_SIZE, &mut buffer[0..size])?;
					return Ok(&buffer[0..size]);
				}
			}
			FindResult::NotFound(_) => {
				return Err(FlashStoreError::NotFound);
			}
		}
	}

	/** consumes 1kb of stack space */
	fn used_space_except(&mut self, file_number: Option<u8>) -> Result<usize, FlashStoreError> {
		let mut sizes = [0; 255];

		let mut position = 0;
		while position < Flash::SIZE {
			let (number, size) = self.read_header(position)?;

			if number == 0xFF {
				// we read an uninitialized header, i.e. we hit end-of-storage
				break;
			}

			if position + HEADER_SIZE + size > Flash::SIZE {
				// the file exceeds the flash size
				return Err(FlashStoreError::CorruptData);
			}

			sizes[number as usize] = Self::round(HEADER_SIZE + size);
			position += Self::round(HEADER_SIZE + size);
		}

		if let Some(file_number) = file_number {
			sizes[file_number as usize] = 0;
		}

		return Ok(sizes.iter().sum());
	}

	/** consumes 1kb of stack space */
	pub fn used_space(&mut self) -> Result<usize, FlashStoreError> {
		self.used_space_except(None)
	}

	// consumes more than 1kb of stack storage
	fn generate_file_index(&mut self) -> Result<[usize; 255], FlashStoreError> {
		let mut positions = [usize::MAX; 255];

		let mut position = 0;
		while position < Flash::SIZE {
			let (number, size) = self.read_header(position)?;

			if number == 0xFF {
				// we read an uninitialized header, i.e. we hit end-of-storage
				break;
			}

			if position + HEADER_SIZE + size > Flash::SIZE {
				// the file exceeds the flash size
				return Err(FlashStoreError::CorruptData);
			}

			positions[number as usize] = position;
			position += Self::round(HEADER_SIZE + size);
		}

		return Ok(positions);
	}

	// consumes more than 1024 + PAGE_SIZE bytes of stack storage
	fn compact_flash_except(&mut self, except_file_number: u8) -> Result<usize, FlashStoreError> {
		use core::convert::TryInto;
		let mut page_buffer = [0u8; PAGE_SIZE];

		let mut read_pointer = 0;
		let mut write_pointer = 0;
		let mut remaining_bytes_to_copy = 0;

		let file_index = self.generate_file_index()?;

		for page in (0..Flash::SIZE).step_by(Flash::PAGE_SIZE) {
			self.flash.read(page, &mut page_buffer)?;
			self.flash.erase_page(page)?;

			if remaining_bytes_to_copy > 0 {
				let copy_from_this_page = remaining_bytes_to_copy.min(Flash::PAGE_SIZE);
				self.flash.write(write_pointer, &page_buffer[0..copy_from_this_page])?;
				write_pointer += copy_from_this_page;
				remaining_bytes_to_copy -= copy_from_this_page;
			}

			while read_pointer < page + Flash::PAGE_SIZE {
				assert!(remaining_bytes_to_copy == 0);
				let read_pointer_in_page = read_pointer - page;
				let remaining_page = &page_buffer[read_pointer_in_page..];

				let (file_number, file_size) = Self::parse_header(remaining_page[0..HEADER_SIZE].try_into().unwrap());

				if file_number == 0xFF {
					read_pointer = Flash::SIZE;
					break;
				}

				let entry_size = Self::round(HEADER_SIZE + file_size);
				let entry_size_on_this_page = entry_size.min(remaining_page.len());
				let entry_size_on_next_page = entry_size - entry_size_on_this_page;

				let discard_file_entry =
					file_index[file_number as usize] != read_pointer ||
					file_number == except_file_number;

				if !discard_file_entry {
					self.flash.write(write_pointer, &remaining_page[0..entry_size_on_this_page])?;
					write_pointer += entry_size_on_this_page;
					remaining_bytes_to_copy = entry_size_on_next_page;
				}
				read_pointer += entry_size;
			}
		}

		Ok(write_pointer)
	}

	/** consumes at least 1024 + PAGE_SIZE bytes of stack in the worst case. */
	pub fn write_file(&mut self, file_number: u8, buffer: &[u8]) -> Result<(), FlashStoreError> {
		assert!(file_number != 0xFF, "Illegal file number 0xFF");
		
		let mut end_of_store = self.end_of_store()?;

		if end_of_store + HEADER_SIZE + buffer.len() > Flash::SIZE {
			if HEADER_SIZE + buffer.len() > Flash::SIZE - self.used_space_except(Some(file_number))? {
				return Err(FlashStoreError::NoSpaceLeft);
			}
			end_of_store = self.compact_flash_except(file_number)?;
		}

		let header = Self::make_header(file_number, buffer.len());
		if Flash::WORD_SIZE > HEADER_SIZE {
			// Let's just assume that no flash has a greater word size than 32.
			// This is a limitation of Rust currently, this array should have a size of
			// Flash::WORD_SIZE instead. FIXME
			let mut temp = [0u8; 32];
			let word_buffer = &mut temp[0..Flash::WORD_SIZE];

			word_buffer[0..HEADER_SIZE].copy_from_slice(&header);
			if Flash::WORD_SIZE < buffer.len() + HEADER_SIZE {
				word_buffer[HEADER_SIZE..].copy_from_slice(&buffer[0..(Flash::WORD_SIZE-HEADER_SIZE)]);
				self.flash.write(end_of_store, &word_buffer)?;
				self.flash.write(end_of_store + Flash::WORD_SIZE, &buffer[(Flash::WORD_SIZE-HEADER_SIZE)..])?;
			}
			else {
				word_buffer[HEADER_SIZE..(HEADER_SIZE + buffer.len())].copy_from_slice(buffer);
				self.flash.write(end_of_store, &word_buffer)?;
			}
		}
		else {
			self.flash.write(end_of_store, &header)?;
			self.flash.write(end_of_store + header.len(), buffer)?;
		}

		Ok(())
	}

	pub fn initialize_flash(&mut self) -> Result<(), FlashStoreError> {
		for page in (0..Flash::SIZE).step_by(Flash::PAGE_SIZE) {
			self.flash.erase_page(page)?;
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::FlashTrait;
	use super::FlashStore;
	use super::FlashStoreError;
	use super::FlashAccessError;
	use super::std::vec::Vec;

	pub struct XorShift {
		state: u64
	}

	impl Iterator for XorShift {
		type Item = u64;
		fn next(&mut self) -> Option<u64> {
			self.state ^= self.state << 13;
			self.state ^= self.state >> 17;
			self.state ^= self.state << 5;
			Some(self.state)
		}
	}

	pub fn rand_iter(seed: u32) -> XorShift {
		let mut iter = XorShift { state: seed as u64 | (((seed ^ 0xDEADBEEF) as u64) << 32) };
		for _ in 0..16 {
			iter.next();
		}
		return iter;
	}

	trait FlashTraitExt {
		fn erase_count(&self) -> usize;
		fn new() -> Self;
	}

	macro_rules! flash_impl {
		($name:ident, $size: literal, $page_size: literal, $word_size: literal, $erased_value: literal) => {
			struct $name {
				data: [u8; $size],
				erase_count: [usize; $size / $page_size]
			}

			impl FlashTraitExt for $name {
				fn erase_count(&self) -> usize {
					let val = self.erase_count[0];
					assert!(self.erase_count.iter().all(|x| *x == val));
					return val;
				}
				
				fn new() -> Self {
					$name {
						data: [$erased_value; $size],
						erase_count: [0; $size / $page_size]
					}
				}
			}

			impl FlashTrait for &mut $name {
				const SIZE: usize = $size;
				const PAGE_SIZE: usize = $page_size;
				const WORD_SIZE: usize = $word_size;
				const ERASED_VALUE: u8 = $erased_value;

				fn erase_page(&mut self, page: usize) -> Result<(), FlashAccessError> {
					assert!(page % Self::PAGE_SIZE == 0);
					self.erase_count[page / Self::PAGE_SIZE] += 1;
					self.data[page..(page+Self::PAGE_SIZE)].copy_from_slice(&[Self::ERASED_VALUE; Self::PAGE_SIZE]);
					Ok(())
				}

				fn read(&mut self, address:usize, data: &mut [u8]) -> Result<(), FlashAccessError> {
					data.copy_from_slice(&self.data[address..(address+data.len())]);
					Ok(())
				}
				fn write(&mut self, address: usize, data: &[u8]) -> Result<(), FlashAccessError> {
					assert!(address % Self::WORD_SIZE == 0);
					self.data[address..(address+data.len())].copy_from_slice(data);
					Ok(())
				}
			}
		}
	}

	#[test]
	fn gracefully_fails_for_corrupt_data() {
		flash_impl!(MyFlash, 1024, 128, 4, 0xFF);
		let mut flash = MyFlash::new();

		flash.data[0] = 42; // file number 42
		flash.data[1] = 253; // size 1021 (which, together with the header, exceeds the flash size)
		flash.data[2] = 3;
		flash.data[3] = 0;

		let mut store = FlashStore::<_, 128>::new(&mut flash);
		let mut buffer = [0; 2048];

		assert!(store.read_file(42, &mut buffer) == Err(FlashStoreError::CorruptData));
		assert!(store.read_file(1, &mut buffer) == Err(FlashStoreError::CorruptData));
		assert!(store.write_file(42, &buffer[0..1]) == Err(FlashStoreError::CorruptData));
		assert!(store.write_file(1, &buffer[0..1]) == Err(FlashStoreError::CorruptData));
	}

	#[test]
	fn gracefully_fails_for_corrupt_data2() {
		flash_impl!(MyFlash, 1024, 128, 4, 0xFF);
		let mut flash = MyFlash::new();

		flash.data[0] = 1; // file number 1
		flash.data[1] = 4; // size 4
		flash.data[2] = 0;
		flash.data[3] = 0;

		flash.data[8] = 42; // file number 42
		flash.data[9] = 245; // size 1023 (which, together with the previous file and the header, exceeds the flash size)
		flash.data[10] = 3;
		flash.data[11] = 0;

		let mut store = FlashStore::<_, 128>::new(&mut flash);
		let mut buffer = [0; 2048];

		assert!(store.read_file(42, &mut buffer) == Err(FlashStoreError::CorruptData));
		assert!(store.read_file(1, &mut buffer) == Err(FlashStoreError::CorruptData));
		assert!(store.read_file(2, &mut buffer) == Err(FlashStoreError::CorruptData));
		assert!(store.write_file(42, &buffer[0..1]) == Err(FlashStoreError::CorruptData));
		assert!(store.write_file(1, &buffer[0..1]) == Err(FlashStoreError::CorruptData));
		assert!(store.write_file(2, &buffer[0..1]) == Err(FlashStoreError::CorruptData));
	}

	#[test]
	fn overwrite_flash_filling_file() {
		flash_impl!(MyFlash, 1024, 128, 4, 0xFF);
		let mut flash = MyFlash::new();

		flash.data[0] = 42; // file number 42
		flash.data[1] = 252; // size 1020 (which, together with the header, is exactly the flash size)
		flash.data[2] = 3;
		flash.data[3] = 0;

		let mut store = FlashStore::<_, 128>::new(&mut flash);
		let mut buffer = [0; 2048];

		assert!(store.read_file(42, &mut buffer).is_ok());

		match store.write_file(11, &buffer[0..1]) {
			Err(FlashStoreError::NoSpaceLeft) => {}
			_ => {panic!()}
		}

		assert!(store.write_file(42, &buffer[0..1]).is_ok());
		assert!(store.write_file(11, &buffer[0..1]).is_ok());
	}

	#[test]
	fn stress_test_congestion_1() {
		flash_impl!(MyFlash, 1024, 128, 1, 0xFF);
		stress_test::<MyFlash, 255, 300>();
	}

	#[test]
	fn stress_test_lots_of_small_files_1() {
		flash_impl!(MyFlash, 1024, 128, 1, 0xFF);
		stress_test::<MyFlash ,100, 4>();
	}

	#[test]
	fn stress_test_lots_of_overwritten_files_1() {
		flash_impl!(MyFlash, 1024, 128, 1, 0xFF);
		stress_test::<MyFlash, 5, 100>();
	}

	#[test]
	fn stress_test_multipage_1() {
		flash_impl!(MyFlash, 1024, 128, 1, 0xFF);
		stress_test::<MyFlash, 3, 500>();
	}

	#[test]
	fn stress_test_congestion_2() {
		flash_impl!(MyFlash, 1024, 128, 2, 0xFF);
		stress_test::<MyFlash, 255, 300>();
	}

	#[test]
	fn stress_test_lots_of_small_files_2() {
		flash_impl!(MyFlash, 1024, 128, 2, 0xFF);
		stress_test::<MyFlash ,100, 4>();
	}

	#[test]
	fn stress_test_lots_of_overwritten_files_2() {
		flash_impl!(MyFlash, 1024, 128, 2, 0xFF);
		stress_test::<MyFlash, 5, 100>();
	}

	#[test]
	fn stress_test_multipage_2() {
		flash_impl!(MyFlash, 1024, 128, 2, 0xFF);
		stress_test::<MyFlash, 3, 500>();
	}

	#[test]
	fn stress_test_congestion_4() {
		flash_impl!(MyFlash, 1024, 128, 4, 0xFF);
		stress_test::<MyFlash, 255, 300>();
	}

	#[test]
	fn stress_test_lots_of_small_files_4() {
		flash_impl!(MyFlash, 1024, 128, 4, 0xFF);
		stress_test::<MyFlash ,100, 4>();
	}

	#[test]
	fn stress_test_lots_of_overwritten_files_4() {
		flash_impl!(MyFlash, 1024, 128, 4, 0xFF);
		stress_test::<MyFlash, 5, 100>();
	}

	#[test]
	fn stress_test_multipage_4() {
		flash_impl!(MyFlash, 1024, 128, 4, 0xFF);
		stress_test::<MyFlash, 3, 500>();
	}

	#[test]
	fn stress_test_congestion_16() {
		flash_impl!(MyFlash, 1024, 128, 16, 0xFF);
		stress_test::<MyFlash, 255, 300>();
	}

	#[test]
	fn stress_test_lots_of_small_files_16() {
		flash_impl!(MyFlash, 1024, 128, 16, 0xFF);
		stress_test::<MyFlash ,100, 4>();
	}

	#[test]
	fn stress_test_lots_of_overwritten_files_16() {
		flash_impl!(MyFlash, 1024, 128, 16, 0xFF);
		stress_test::<MyFlash, 5, 100>();
	}

	#[test]
	fn stress_test_multipage_16() {
		flash_impl!(MyFlash, 1024, 128, 16, 0xFF);
		stress_test::<MyFlash, 3, 500>();
	}

	fn stress_test<MyFlash, const MAX_FILE_NUMBER: u64, const MAX_FILE_SIZE: usize>()
	where for<'a> &'a mut MyFlash : FlashTrait, MyFlash: FlashTraitExt
	{
		let mut flash = MyFlash::new();
		let mut files : Vec<Option<Vec<u8>>> = std::iter::repeat(None).take(255).collect::<Vec<_>>();
		let mut store = FlashStore::<_, 128>::new(&mut flash);
		let mut buf = [0; 1024];

		store.initialize_flash().unwrap();

		let mut used = 0;
		let granularity = <&mut MyFlash>::WORD_SIZE.max(4);

		for i in rand_iter(42).take(3000) {
			// check
			for (i, f) in files.iter().enumerate() {
				match store.read_file(i as u8, &mut buf) {
					Ok(result) => {
						assert!(f.as_ref().unwrap().as_slice() == result)
					}
					Err(FlashStoreError::NotFound) => {
						assert!(f.is_none());
					}
					Err(_) => {
						panic!(); // GRCOV_EXCL_LINE
					}
				}
			}

			// modify
			let file_number = i % MAX_FILE_NUMBER;
			let mut rng = rand_iter((i & 0xFFFFFFFF) as u32);
			let len = rng.next().unwrap() as usize % MAX_FILE_SIZE;
			let data = rng.take(len).map(|x| (x & 0xFF) as u8).collect::<Vec<_>>();
			let entry_size = (len + 4 + granularity - 1) / granularity * granularity;

			let result = store.write_file(file_number as u8, &data);

			if let Some(error) = result.err() {
				match error {
					FlashStoreError::NoSpaceLeft => {}
					_ => {panic!()} // GRCOV_EXCL_LINE
				}
			}

			let prev_entry_size = if let Some(ref f) = files[file_number as usize] {
				(f.len() + 4 + granularity - 1) / granularity * granularity
			}
			else {
				0
			};

			if used - prev_entry_size + entry_size > 1024 {
				assert!(result.is_err());
			}
			else {
				assert!(result.is_ok());
				files[file_number as usize] = Some(data);
				used = used - prev_entry_size +  entry_size;
			}
		}

		assert!(flash.erase_count() >= 5);
	}
}
