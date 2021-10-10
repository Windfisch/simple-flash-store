fn round_up(val: usize, granularity: usize) -> usize {
	((val - 1) / granularity + 1) * granularity
}

fn round_down(val: usize, granularity: usize) -> usize {
	val / granularity * granularity
}

pub trait FlashTrait {
	/// The flash size in bytes. Must be a multiple of `page_size`.
	const size: usize;

	/// The granularity in bytes with which pages can be erased
	const page_size: usize;

	/// The granularity in bytes with which data can be written. A value of 3 is unsupported.
	const word_size: usize;

	/// Value of the first byte of each erased word. usually 0xFF
	const erased_value: u8;

	type Error;

	/// Erases the page starting at `address`. `address` must be a multiple of `page_size`
	fn erase_page(&mut self, address: usize) -> Result<(), Self::Error>;

	/// Writes `data` to `address`. `address` must be a multiple of `word_size`.
	/// If `data.len()` is not a multiple of `word_size`, undefined padding is added.
	fn write(&mut self, address: usize, data: &[u8]) -> Result<(), Self::Error>;

	/// Reads `data.len()` bytes from `address`. neither `address` nor `data.len()` need to be multiples of `word_size`.
	fn read(&mut self, address: usize, data: &mut [u8]) -> Result<(), Self::Error>;
}
