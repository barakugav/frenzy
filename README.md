# frenzy
A Rust implementation of the [One Billion Row Challenge](https://github.com/gunnarmorling/1brc).

```bash
git clone https://github.com/barakugav/frenzy.git
cd frenzy

git clone https://github.com/gunnarmorling/1brc.git
cd 1brc
./mvnw clean verify
./create_measurements.sh 1000000000
cd ..

RUSTFLAGS=-Ctarget-cpu=native cargo build --release
./target/release/frenzy 1brc/measurements.txt
```

The challenge is to process an input file with 1 billion rows, each in the format `<string: station name>;<double: measurement>\n`, and produce a summary of min/avg/max measurements per station.

Why this implementation is fast?
- Uses mmap to read the input file

- Multi threaded
    <br> We split the input file into chunks and process each chunk in a separate thread, merging the results at the end is trivial in this case.

- Batched processing
    <br> Each worker thread that processes a chunk of the input file further splits it into smaller (4) segments, maintaining a cursor per segment.
    In each iteration the worker parses multiple lines (one from each segment) before moving the cursors forward.
    Every variable that you expect to be a scalar in the main loop such as `u32` is actually `[u32; 4]`.
    This helps utilizing the CPU execution units better, as there are many data dependencies in parsing a single line.

- Simd
    <br> When possible, we use simd operations to process multiple values in parallel.
    For example, when searching for a semicolon delimiter at the end of a station name, we load 8 bytes at a time and compare them to `;` in parallel.
    ```rust
    let semicolon_pos =
        Simd::<u8, _>::from_array(next_bytes)
            .simd_eq(Simd::splat(b';'))
            .to_bitmask()
            .trailing_zeros() as usize;
    ```
- Custom hash map
    <br> Instead of using `std::collections::HashMap`, we implement our own simplified hash map that is optimized for this specific use case.
    A contiguous array of buckets is used, accessed as `buckets[hash % capacity]`, without support for collision resolution.
    When a collision occurs, we simply store the key-value pair in a fallback std hashmap, gated behind a `#[inline(never)] #[cold]` function to avoid polluting the main processing loop.
    A sufficiently large capacity (`128 * expected_station_num`) is chosen to minimize collisions, and there are usually 0-2 such collisions in practice per run.
    No resizing of the hash map is done at runtime.

- Custom hash function
    <br> Given a station name bytes, we split it into chunks of 8 bytes, convert each chunk to a u64 and xor them together to produce a hash.
    The last chunk is padded with zeros if needed.
    Some mixing is done at the end to have better distribution in the lower bits.

- Custom string type used for station names
    <br> Instead of using `&str` or `String` for station names, we define a custom `StationName` struct that stores the first 16 bytes of the name in a `u128`, and the remainder as a pointer and length.
    This allows for faster comparisons and hashing, as many station names are short and fit within 16 bytes.
    It also avoid utf-8 validation overhead.
    ```rust
    struct StationName {
        // The first 16 bytes of the name, might be padded with zeros
        prefix: u128,
        // `(name as *const u8) + 16`
        remainder_ptr: *const u8,
        // `name.len() as isize - 16`, might be negative
        remainder_len: isize,
    }
    ```

- No allocations in the main processing loop

- Explicit use of compiler hints, such as `#[inline]`, `#[cold]`, `std::hint::likely`, etc.

- Use of small integer types instead of floats where possible

- Custom branchless parsing of float temperatures

- No bound checks
    <br> When reading, we sometime read the next 8 or 16 bytes, without checking if we are at the end of the input.
    To avoid UB, we first split the input to a main body and a tail of at least 128 bytes, where the main body is processed safely without bound checks, and the tail is processed at the end with trivial unoptimized safe code.
    In addition, raw pointers are preferred over slices almost everywhere to avoid bound checks.

Things that can be improved further:
- The braceless float parsing can be further optimized using simd operations and bit manipulations.
  Its implementation branchless, but not very efficient.
- Hardware specific hyper parameters tuning, such as SIMD width, batch size (currently 4), etc.
