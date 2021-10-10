#[cfg(test)]
extern crate std;

pub mod flash_trait;
use flash_trait::FlashTrait;

pub struct FlashStore<Flash: FlashTrait, const page_size: usize> {
	flash: Flash
}

#[derive(Debug, Clone, Copy)]
pub enum FlashStoreError {
	NotFound,
	BufferTooSmall,
	CorruptData,
	NoSpaceLeft,
}

enum FindResult {
	/// holds the header offset and size of the file
	Found(usize, usize),

	/// holds the end-of-storage pointer
	NotFound(usize)
}

const HEADER_SIZE: usize = 4;

impl<Flash: FlashTrait, const page_size: usize> FlashStore<Flash, page_size> {
	pub fn new(flash: Flash) -> FlashStore<Flash, page_size> {
		assert!(Flash::word_size != 3, "A word size of 3 is unsupported");
		assert!(Flash::page_size == page_size);
		FlashStore { flash }
	}
	
	fn parse_header(header: [u8; 4]) -> (u8, usize) {
		let number = (!header[0]) ^ Flash::erased_value;
		let size = u32::from_le_bytes([header[1], header[2], header[3], 0]);
		(number, size as usize)
	}
	
	fn make_header(number: u8, size: usize) -> [u8; 4] {
		assert!(size <= 0xFFFFFF);
		let s = (size as u32).to_le_bytes();
		[(!number) ^ Flash::erased_value, s[0], s[1], s[2]]
	}


	fn round(offset: usize) -> usize {
		let word_size = Flash::word_size.max(HEADER_SIZE);
		((offset - 1) / word_size + 1) * word_size
	}

	fn read_header(&mut self, position: usize) -> Result<(u8, usize), FlashStoreError> {
		let mut header: [u8; 4] = [0,0,0,0];
		self.flash.read(position, &mut header);
		Ok(Self::parse_header(header))
	}

	fn find(&mut self, file_number: Option<u8>) -> Result<FindResult, FlashStoreError> {
		use FlashStoreError::*;

		let file_number = file_number.unwrap_or(0xFF);
		let mut position = 0;
		let mut found = None;

		while position < Flash::size {
			let (number, size) = self.read_header(position)?;

			if number == 0xFF {
				// we read an uninitialized header, i.e. we hit end-of-storage
				break;
			}
			else if number == file_number {
				found = Some((position, size));
			}

			if position + HEADER_SIZE + size > Flash::size {
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
			unreachable!();
		}
	}

	/// file_number must not be 0xFF
	pub fn read_file<'a>(&mut self, file_number: u8, buffer: &'a mut [u8]) -> Result<&'a [u8], FlashStoreError> {
		assert!(file_number != 0xFF, "Illegal file number 0xFF");

		match self.find(Some(file_number))? {
			FindResult::Found(position, size) => {
				self.flash.read(position + HEADER_SIZE, &mut buffer[0..size]);
				return Ok(&buffer[0..size]);
			}
			FindResult::NotFound(_) => {
				return Err(FlashStoreError::NotFound);
			}
		}
	}

	fn used_space_except(&mut self, file_number: u8) -> Result<usize, FlashStoreError> {
		let mut sizes = [0; 255];

		let mut position = 0;
		while position < Flash::size {
			let (number, size) = self.read_header(position)?;

			if number == 0xFF {
				// we read an uninitialized header, i.e. we hit end-of-storage
				break;
			}

			if position + HEADER_SIZE + size > Flash::size {
				// the file exceeds the flash size
				return Err(FlashStoreError::CorruptData);
			}

			sizes[number as usize] = Self::round(HEADER_SIZE + size);
			position += Self::round(HEADER_SIZE + size);
		}

		sizes[file_number as usize] = 0;

		return Ok(sizes.iter().sum());
	}

	fn generate_file_index(&mut self) -> Result<[usize; 255], FlashStoreError> {
		let mut positions = [usize::MAX; 255];

		let mut position = 0;
		while position < Flash::size {
			let (number, size) = self.read_header(position)?;

			if number == 0xFF {
				// we read an uninitialized header, i.e. we hit end-of-storage
				break;
			}

			if position + HEADER_SIZE + size > Flash::size {
				// the file exceeds the flash size
				return Err(FlashStoreError::CorruptData);
			}

			positions[number as usize] = position;
			position += Self::round(HEADER_SIZE + size);
		}

		return Ok(positions);
	}

	fn compact_flash_except(&mut self, except_file_number: u8) -> Result<usize, FlashStoreError> {
		println!("compacting the flash except for file {}", except_file_number);
		use core::convert::TryInto;
		let mut page_buffer = [0u8; page_size];

		let mut read_pointer = 0;
		let mut write_pointer = 0;
		let mut remaining_bytes_to_copy = 0;

		let file_index = self.generate_file_index()?;

		for page in (0..Flash::size).step_by(Flash::page_size) {
			self.flash.read(page, &mut page_buffer);
			self.flash.erase_page(page);

			if remaining_bytes_to_copy > 0 {
				let copy_from_this_page = remaining_bytes_to_copy.min(Flash::page_size);
				self.flash.write(write_pointer, &page_buffer[0..copy_from_this_page]);
				write_pointer += copy_from_this_page;
				remaining_bytes_to_copy -= copy_from_this_page;
			}

			while read_pointer < page + Flash::page_size {
				assert!(remaining_bytes_to_copy == 0);
				let read_pointer_in_page = read_pointer - page;
				let remaining_page = &page_buffer[read_pointer_in_page..];

				let (file_number, file_size) = Self::parse_header(remaining_page[0..HEADER_SIZE].try_into().unwrap());

				if file_number == 0xFF {
					read_pointer = Flash::size;
					break;
				}

				let entry_size = Self::round(HEADER_SIZE + file_size);
				let entry_size_on_this_page = entry_size.min(remaining_page.len());
				let entry_size_on_next_page = entry_size - entry_size_on_this_page;

				let discard_file_entry =
					file_index[file_number as usize] != read_pointer ||
					file_number == except_file_number;

				if !discard_file_entry {
					self.flash.write(write_pointer, &remaining_page[0..entry_size_on_this_page]);
					write_pointer += entry_size_on_this_page;
					remaining_bytes_to_copy = entry_size_on_next_page;
				}
				read_pointer += entry_size;
			}
		}

		Ok(write_pointer)
	}

	pub fn write_file(&mut self, file_number: u8, buffer: &[u8]) -> Result<(), FlashStoreError> {
		assert!(file_number != 0xFF, "Illegal file number 0xFF");
		
		let mut end_of_store = self.end_of_store()?;

		if end_of_store + HEADER_SIZE + buffer.len() > Flash::size {
			println!("used: {}", self.used_space_except(file_number)?);
			if HEADER_SIZE + buffer.len() > Flash::size - self.used_space_except(file_number)? {
				return Err(FlashStoreError::NoSpaceLeft);
			}
			end_of_store = self.compact_flash_except(file_number)?;
		}

		let header = Self::make_header(file_number, buffer.len());
		if Flash::word_size > HEADER_SIZE {
			// Let's just assume that no flash has a greater word size than 32.
			// This is a limitation of Rust currently, this array should have a size of
			// Flash::word_size instead. FIXME
			let mut temp = [0u8; 32];
			let word_buffer = &mut temp[0..Flash::word_size];

			word_buffer[0..HEADER_SIZE].copy_from_slice(&header);
			word_buffer[HEADER_SIZE..].copy_from_slice(&buffer[0..(Flash::word_size-HEADER_SIZE)]);
			self.flash.write(end_of_store, &word_buffer);
			self.flash.write(end_of_store + Flash::word_size, &buffer[(Flash::word_size-HEADER_SIZE)..]);
		}
		else {
			self.flash.write(end_of_store, &header);
			self.flash.write(end_of_store + header.len(), buffer);
		}

		Ok(())
	}
}

mod tests {
	use super::flash_trait::FlashTrait;
	use super::FlashStore;
	use super::FlashStoreError;

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


	#[test]
	fn stress_test_congestion() {
		stress_test::<255, 300>();
	}

	#[test]
	fn stress_test_lots_of_small_files() {
		stress_test::<100, 4>();
	}

	#[test]
	fn stress_test_lots_of_overwritten_files() {
		stress_test::<5, 100>();
	}

	#[test]
	fn stress_test_multipage() {
		stress_test::<3, 500>();
	}

	fn stress_test<const MAX_FILE_NUMBER: u64, const MAX_FILE_SIZE: usize>() {
		struct MyFlash {
			data: [u8; 1024],
			erase_count: [usize; 8]
		}

		impl MyFlash {
			pub fn erase_count(&self) -> usize {
				let val = self.erase_count[0];
				assert!(self.erase_count.iter().all(|x| *x == val));
				return val;
			}
		}

		impl FlashTrait for &mut MyFlash {
			const size: usize = 1024;
			const page_size: usize = 128;
			const word_size: usize = 4;
			const erased_value: u8 = 0xFF;
			type Error = ();

			fn erase_page(&mut self, page: usize) -> Result<(), ()> {
				assert!(page % Self::page_size == 0);
				self.erase_count[page / Self::page_size] += 1;
				self.data[page..(page+Self::page_size)].copy_from_slice(&[Self::erased_value; Self::page_size]);
				Ok(())
			}

			fn read(&mut self, address:usize, data: &mut [u8]) -> Result<(), ()> {
				data.copy_from_slice(&self.data[address..(address+data.len())]);
				Ok(())
			}
			fn write(&mut self, address: usize, data: &[u8]) -> Result<(), ()> {
				assert!(address % Self::word_size == 0);
				self.data[address..(address+data.len())].copy_from_slice(data);
				Ok(())
			}
		}

		let mut flash = MyFlash { data: [0xFF; 1024], erase_count: [0; 8] };
		let mut files : Vec<Option<Vec<u8>>> = std::iter::repeat(None).take(255).collect::<Vec<_>>();
		let mut store = FlashStore::<_, 128>::new(&mut flash);
		let mut buf = [0; 1024];

		let mut used = 0;
		let granularity = 4;

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
						panic!();
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
					_ => {panic!()}
				}
			}

			println!("Writing {} bytes (plus header) into file {}, with {} bytes already used", len, file_number, used);

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

		assert!(flash.erase_count() >= 4);
	}
}
