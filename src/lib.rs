pub mod constants;
pub mod tests;
pub mod type_guesser;

use constants::*;
use ethers::types::{U128, U256};
use type_guesser::*;

// ------------------------------------------------------------
//  Helpers
// ------------------------------------------------------------

/// Converts `calldata` into chunks of `size`.
pub fn chunkify(calldata: &str, size: usize) -> Vec<String> {
    calldata
        .chars()
        .collect::<Vec<char>>()
        .chunks(size)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<String>>()
}

/// Adds padding of '0's.
///
/// ## Params
/// 1. chunks - vector of bytes-32 (64 chars).
/// 2. current - the chunks element we're currently on.
/// 3. side - front or back of the calldata (true == left, false = right).
pub fn add_padding(chunks: Vec<String>, current: usize, side: bool) -> Vec<String> {
    // let total = current * 64;
    let mut chunks = chunks.clone();
    match side {
        true => chunks[current] = format!("{}{}", EMPTY_4.to_string(), chunks[current]),
        false => chunks[current] = format!("{}{}", chunks[current], EMPTY_4.to_string()),
    }
    let len = chunks.len() - 1;
    chunks[len] = chunks[len].split_at(56).0.to_string();
    let new = chunkify(&chunks.concat(), 64);
    new
}

/// Attempts to a selector from the bytes-32 (64 &str).
///
/// ## Returns:
/// 1. Function selector.
/// 2. New calldata param.
pub fn try_parse_selector(calldata: &str) -> (String, String) {
    let mut chunks = chunkify(calldata, 8);
    // Replace function selector if exists.
    if chunks[0] != EMPTY_4 && chunks[1] == EMPTY_4 && chunks[0] != MASK_4 {
        let selector = chunks[0].clone();
        chunks[0] = chunks[0].replace(&chunks[0], "");
        return (selector, chunks.join(""));
    }
    (EMPTY_4.to_string(), chunks.join(""))
}

/// Moves EMPTY_4 to end of calldata.
///
/// ## Params
/// 1. chunks: Vec<String64>.
///
/// ## Returns:
/// 1. New chunks: Vec<String64>.
/// 2. New param for index `from`.
pub fn rearrange_chunks(
    chunks: Vec<String>,
    from: usize,
    replacement: String,
) -> (Vec<String>, String) {
    let mut new_chunks = chunks.clone();
    new_chunks[from] = replacement;
    // TODO...Add selector replacement offset.
    // ...
    let new_calldata = format!("{}{}", new_chunks.concat(), EMPTY_4);
    let new_chunks = chunkify(&new_calldata, 64);
    (new_chunks, new_calldata)
}

/// Returns the raw param before `current`, if available.
///
/// ## Params
/// 1. chunks - vector of bytes-32 (64 chars).
/// 2. current - the chunks element we're currently on.
pub fn last_raw(params: &Vec<String>, current: usize) -> Option<String> {
    match current == 0 {
        true => None,
        false => Some(params[current - 1].clone()),
    }
}

/// Returns the raw param after `current`, if available.
///
/// ## Params
/// 1. chunks - vector of bytes-32 (64 chars).
/// 2. current - the chunks element we're currently on.
pub fn next_raw(params: &Vec<String>, current: usize) -> Option<String> {
    let len = params.len() - 1;
    match current >= len {
        true => None,
        false => Some(params[len].clone()),
    }
}

/// Guesses the potential types of the parameter by checking specific patterns.
///
/// ## Params
/// 1. param - 32 byte str representation of parameter.
///            e.g, "000000000000000000000000000000000000000000831162ce86bc88052f80fd"
///
/// ## Returns
/// 1. All potential types the parameter can be.
pub fn guess_param_type(param: &str) -> ParamTypes {
    // Quick check for maxed out types.
    match param {
        EMPTY_32 => return ParamTypes::new(vec![Types::AnyZero]),
        MAX_U128 => return ParamTypes::new(vec![Types::MaxUint128]),
        MAX_U256 => return ParamTypes::new(vec![Types::AnyMax]),
        _ => {}
    }

    // Break param into 4 byte sections.
    let chunks = chunkify(param, 8);

    // Selector detection:
    // if: !00000000... && !FFFFFFFF... && ________00000000
    if chunks[0] != EMPTY_4 && chunks[0] != MASK_4 && chunks[1] == EMPTY_4 {
        return ParamTypes::new(vec![Types::Selector, Types::String, Types::Bytes]);
    }

    // Check if it's an Int by: if FFFFFFFF
    // Ints replace 0s with 1s in bitwise
    if chunks[0] == MASK_4 {
        // if: FFFFFFFFFFFFFFFF we can assume it's an Int
        match chunks[1] == MASK_4 {
            true => return ParamTypes::new(vec![Types::Int]),
            false => return ParamTypes::new(vec![Types::Int, Types::String, Types::Bytes]),
        }
    }

    // Check if we found an address:
    // Todo:
    // - Check for optimised addresses via heuristics
    let trimmed = param.trim_start_matches('0').to_string();
    if trimmed.len() == 40 {
        return ParamTypes::new(vec![Types::Address, Types::Bytes20, Types::Uint]);
    }

    // If the value can be converted to U256
    if let Ok(v) = U256::from_str_radix(&param, 16) {
        // If value is 0 or 1.
        if v <= U256::one() {
            return ParamTypes::new(vec![Types::Uint8, Types::Bytes1, Types::Bool]);
        }

        // If value is of type `uint8`.
        if v <= U256::from_dec_str("8").unwrap() {
            return ParamTypes::new(vec![Types::Uint8, Types::Bytes1]);
        }
    }

    // Eliminated some patterns; now we can conclude it can be one of these.
    ParamTypes::new(vec![Types::Uint, Types::Int, Types::Bytes])
}

// ------------------------------------------------------------
//  Calldata
// ------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Calldata {
    /// Raw calldata being assessed.
    pub calldata: String,
    /// Method selector being targeted.
    pub selector: String,
    /// TODO...IMPLEMENT.
    /// These aren't computed with `nested_details`
    /// The types of each parameter in the initial method being called (`selector`).
    pub main_details: Vec<Params>,
    /// The params found after selector is sliced out.
    raw_params: Vec<String>,
    ///
    params: Vec<String>,
    /// Method calls extending from our method.
    /// Includes potential types guessed.
    nested_details: Vec<Params>,
}

impl Calldata {
    pub fn new(calldata: &str) -> Self {
        let mut s = Self {
            calldata: calldata.to_string(),
            selector: String::new(),
            main_details: vec![],
            raw_params: vec![],
            params: vec![],
            nested_details: vec![],
        };
        s.parse_selector();
        s.parse_raw_params();
        s.guess_param_types();
        s
    }

    pub fn print(&self) {
        println!("---------- Params ----------");
        // println!("Raw calldata:");
        println!("Method ID: {}", &self.selector);
        println!("Raw Params {:#?}", &self.raw_params);
        println!("Params: {:#?}", &self.params);
        println!("Parsed Params: {:#?}", &self.nested_details);
    }

    /// Parses the method selector the calldata is being sent to.
    /// Prepares the raw calldata params to be parsed.
    pub fn parse_selector(&mut self) {
        // Remove prefix.
        if self.calldata.contains("0x") {
            self.calldata = self.calldata.replace("0x", "");
        }

        // If calldata is of even length.
        if self.calldata.len() % 64 == 0 {
            // Separate calldata into 32-byte chunks.
            self.raw_params = chunkify(&self.calldata, 64);
            // Get function selector from calldata.
            self.selector = self.raw_params[0].split_at(8).0.to_string();
            // Replace it with 0s to just have input.
            self.raw_params[0] = self.raw_params[0].replace(&self.selector, "");
        }
        // Else, calldata is of odd length.
        else {
            // Separate calldata into 1-byte chunks.
            let mut chunks = chunkify(&self.calldata, 2);

            // Create selector.
            self.selector = format!("{}{}{}{}", chunks[0], chunks[1], chunks[2], chunks[3]);

            // Clean chunks.
            for i in 0..=3 {
                chunks[i] = "".to_string();
            }

            let mut params: Vec<String> = vec![String::new()];
            for chunk in chunks.iter() {
                let mut len = params.len() - 1;
                // Check if we have the param.
                if params[len].len() == 64 {
                    // Add new param.
                    params.push(String::new());
                    // Make sure we're pushing to new param.
                    len += 1;
                }
                params[len].push_str(chunk);
            }
            self.raw_params = params;
        }
    }

    /// Parses the raw calldata params for each param and for any new method selectors.
    pub fn parse_raw_params(&mut self) {
        let mut i = 0;
        let mut params: (Vec<String>, bool) = (self.raw_params.clone(), false);
        let mut skipping = 0;

        // TODO...CREATE OFFSET STRUCT
        // TODO...CREATE PC counter/offset identifier for when we reach it to set length
        // ...
        // - PC of offset (e.g.2nd param)
        // - Offset value (e.g. 0x40)
        // - Length       (e.g. 0x02); Default 0 until we reach the offset
        let mut offsets: Vec<(usize, U128, usize)> = vec![]; // pc of offset + offset

        loop {
            // println!("{} Parsed params: {:#?}", i, params.0);
            if skipping != 0 {
                i += skipping;
                skipping = 0;
            }

            if &params.0[i] == EMPTY_32 {
                params.0 = add_padding(params.0, i, true);
                i += 1;
            }

            let raw_param = &params.0[i];
            let trimmed = raw_param.trim_start_matches('0').to_string();

            // Check if param has selector in it.
            let parsed = try_parse_selector(&raw_param);

            // If selector found.
            if parsed.0 != EMPTY_4 && parsed.0 != MASK_4 {
                // println!("selector {}", parsed.0);

                // Check if last param was a length type.
                // They indicate the start of a dynamic type (string, bytes, or array).
                if let Some(last) = last_raw(&params.0, i) {
                    // Trim the last param.
                    let last_trimmed = last.trim_start_matches('0').to_string();
                    if let Ok(v) = U128::from_str_radix(&last_trimmed, 16) {
                        // Extract selector + params.
                        if let Some(skip) = self.parse_len(&params.0, i, v.as_usize()) {
                            // println!("selector found");
                            let rearranged = rearrange_chunks(params.0, i, parsed.1);
                            params = (rearranged.0.clone(), true);

                            // How many chars we skip next loop.
                            skipping = skip;
                        }
                    }
                }
            }
            // Offsets/lengths never have selectors
            // Therefore, we check common offset/length sizes.
            else if trimmed.len() <= 4 {
                // Check if value is for dynamic type.
                if let Ok(v) = U128::from_str_radix(&trimmed, 16) {
                    // Check if offset by checking if
                    // - below safety net length, since they probably wont go that high.
                    // - divisible by 32 bytes (0x20).
                    if v < U128::from(i * 64 + 1920) && v % 64 == U128::from(0) {
                        offsets.push((i, v / 64, 0));
                    }
                }
            }

            // println!("params: {}/{} - {:#?}", i, self.raw_params.len(), params.0);
            i += 1;
            if i == self.raw_params.len() {
                break;
            }
        }

        self.params = params.0;
    }

    ///
    pub fn parse_len(&mut self, params_64: &Vec<String>, from: usize, len: usize) -> Option<usize> {
        let params = params_64.split_at(from);
        let calldata = params.1.concat();
        let cut = calldata.split_at(len * 2);
        let remainder = (len * 2) % 64;
        // println!("remainder: {}", remainder);
        // println!("len: {}", len);
        // If remainder 8 we know its a function.
        if remainder == 8 {
            let cut = cut.0.split_at(8);
            let new_params = chunkify(cut.1, 64);

            // Record params.
            self.nested_details.push(Params::new(cut.0, new_params));

            // If extracting only function.
            if len == 4 {
                return None;
            }

            // println!("to skip {}", (len - 8) * 2 / 64);
            return Some((len - 8) * 2 / 64);
        }
        // TODO..FINISH THIS OFF
        // How to cut out strings????
        // If remainder is 56, probably a string/fn selector.
        else if remainder == 56 {
            //     let cut = cut.0.split_at(8);
            //     let _new_params = chunkify(cut.1, 64);
        }
        None
    }

    /// Attempts to guess the potential types the param could be.
    pub fn guess_param_types(&mut self) {
        println!("guess param types");

        // If our main method calls other methods:
        if self.nested_details.len() > 0 {
            for params in self.nested_details.iter_mut() {
                let mut types: Vec<ParamTypes> = vec![];

                for param in params.params.iter() {
                    let param_types = guess_param_type(param.as_str());
                    types.push(param_types);
                }

                params.types = types;
            }
        }

        // We try to decode the main body's params
        // e.g. `transferBundle(address from, struct[] bundles, address to)`
        let mut types: Vec<ParamTypes> = vec![];
        for i in 0..self.params.len() {
            if i > 0 {
                // if self.params[i]
                unimplemented!();
            }

            let param_types = guess_param_type(i);
            types.push(param_types);
        }

        // params.types = types;
    }

    /// Detects if the parameter is an offset.
    /// Note: An offset means where the word starts from the start of that word.
    ///
    /// ## Returns
    /// 1. Option if a potential length was found.
    ///
    /// ## Example:
    ///
    /// [0] 0000000000000000000000000000000000000000000000000000000000000020
    /// ^ Indicates the length starts at [2]
    pub fn is_offset(&self, i: usize) -> Option<usize> {
        // Trim padded zeros from value.
        let trimmed = &self.params[i].trim_start_matches('0').to_string();

        // Offsets + lengths never have selectors
        // Therefore, we check common offset/length sizes.
        if trimmed.len() <= 4 {
            // Check if value is for dynamic type.
            if let Ok(v) = U128::from_str_radix(&trimmed, 16) {
                // Check if offset by checking if
                // - divisible by 32 bytes (0x20).
                if v % 64 == U128::from(0) {
                    // If 32, the next slot (i) is the length.
                    let to_skip = (v / 32).as_usize();

                    // Make sure offset value exists...
                    let param_len = &self.params.len() - 1;
                    let len_i = i + to_skip;

                    // Does the word's element (i) exist?
                    // E.g. 12 >= 8 (len_i)
                    // E.g. 12 >= 12 (len_i)
                    if param_len >= len_i {
                        // Convert the potential length param to U128.
                        let trimmed_len =
                            &self.params[i + to_skip].trim_start_matches('0').to_string();

                        if let Ok(len) = U128::from_str_radix(&trimmed_len, 16) {
                            let len_v = len.as_usize();

                            // Array detection
                            // If `len_i + len_v` words exists...
                            if param_len >= len_i + len_v {
                                return None;
                            }

                            // String detection
                            if len_v % 2 == 0 {
                                let last_i;

                                // If len_v is 32
                                if len_v > 32 {
                                    // Sanity check of each element besides last one.
                                    for i in 0..len_v - 1 {
                                        // Separate element `i` into 4 byte sections.
                                        let chunks = chunkify(&self.params[len_i + i], 8);

                                        // TODO...EMPTY STRING DETECTION
                                        // Make sure full word isn't empty
                                        if chunks[0] == MASK_4 && chunks[7] == MASK_4 {
                                            return None;
                                        }
                                    }

                                    last_i = len_v;
                                } else {
                                    last_i = len_i + len_v;
                                }

                                // Check remaining bytes of last element.
                                // E.g. 50 % 32 = 18 * 2 = 36
                                let last_element_len = len_v % 32 * 2;
                                // E.g. 36 of 64
                                let padding_amount = 64 - last_element_len;
                                let last_element = &self.params[len_i + last_i];

                                // If padding, check w/ mask on right side.
                                if padding_amount != 0 {
                                    let padding = last_element.split_off(padding_amount);
                                    let mask = "0".repeat(padding_amount);
                                    if padding != mask {
                                        return None;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }
}

#[derive(Clone)]
pub enum DynamicKind {
    String,
    Array,
}

#[derive(Clone)]
pub struct DynamicType {
    kind: DynamicKind,
    offset_pc: usize,
    offset_v: usize,
    length_pc: usize,
    length_v: usize,
}

impl DynamicType {
    pub fn new(
        kind: DynamicKind,
        offset_pc: usize,
        offset_v: usize,
        length_pc: usize,
        length_v: usize,
    ) -> Self {
        Self {
            kind,
            offset_pc,
            offset_v,
            length_pc,
            length_v,
        }
    }
}

/*
cargo test test_calldata -- --nocapture --test-threads=1
*/
#[cfg(test)]
mod test_calldata {
    use super::Calldata;
    /*
        0x5d842074 // fn selector
        000000000000000000000000000000000000000000000006c6b935b8bbd40000 // uint256
        0000000000000000000000000000000000000000000000000000000000000040 // offset of array
        0000000000000000000000000000000000000000000000000000000000000002 // len of array
        00000000000000000000000000000000000000000000002086ac351052600000 // [0] uint256 of array
        00000000000000000000000000000000000000000000002b5e3af16b18800000 // [1] uint256 of array
    */
    #[test]
    // #[ignore]
    fn test_parse_normal_fn() {
        let calldata = "0x5d842074000000000000000000000000000000000000000000000006c6b935b8bbd400000000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000002086ac35105260000000000000000000000000000000000000000000000000002b5e3af16b18800000";
        println!(
            "\nCalldata char len: {}\nBytes: {}",
            calldata.len(),
            calldata.len() / 64 * 32
        );
        let calldata = Calldata::new(calldata);
        calldata.print();
    }

    /*
        Convert the calldata [0]'s etherscan decoded to the following [1]:
        Tx: https://etherscan.io/tx/0x1fe71e209bfed2990ac72e88a640b09008be10579ae1405a8c86ce2ced5767d1
        Calldata: 0xac9650d800000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000001e0000000000000000000000000000000000000000000000000000000000000016488316456000000000000000000000000c011a73ee8576fb46f5e1c5751ca3b9fe0af2a6f000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000000000000000000000000000000000000000002710fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffee530ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff1b18000000000000000000000000000000000000000000000000016345785d89fd6800000000000000000000000000000000000000000000000000007f73eca3063a000000000000000000000000000000000000000000000000016042b530ddaec600000000000000000000000000000000000000000000000000007e59f044bada000000000000000000000000f847e9d51989033b691b8be943f8e9e268f99b9e000000000000000000000000000000000000000000000000000000006377347700000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000412210e8a00000000000000000000000000000000000000000000000000000000
        Function: multicall(bytes[] data)
        [0] MethodID: 0xac9650d8
            [0]:  0000000000000000000000000000000000000000000000000000000000000020
            [1]:  0000000000000000000000000000000000000000000000000000000000000002
            [2]:  0000000000000000000000000000000000000000000000000000000000000040
            [3]:  00000000000000000000000000000000000000000000000000000000000001e0
            [4]:  0000000000000000000000000000000000000000000000000000000000000164
            [5]:  88316456000000000000000000000000c011a73ee8576fb46f5e1c5751ca3b9f
            [6]:  e0af2a6f000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead908
            [7]:  3c756cc200000000000000000000000000000000000000000000000000000000
            [8]:  00002710ffffffffffffffffffffffffffffffffffffffffffffffffffffffff
            [9]:  fffee530ffffffffffffffffffffffffffffffffffffffffffffffffffffffff
            [10]: ffff1b1800000000000000000000000000000000000000000000000001634578
            [11]: 5d89fd6800000000000000000000000000000000000000000000000000007f73
            [12]: eca3063a000000000000000000000000000000000000000000000000016042b5
            [13]: 30ddaec600000000000000000000000000000000000000000000000000007e59
            [14]: f044bada000000000000000000000000f847e9d51989033b691b8be943f8e9e2
            [15]: 68f99b9e00000000000000000000000000000000000000000000000000000000
            [16]: 6377347700000000000000000000000000000000000000000000000000000000
            [17]: 0000000000000000000000000000000000000000000000000000000000000004
            [18]: 12210e8a00000000000000000000000000000000000000000000000000000000
        [1] MethodID: 0xac9650d8
            [00] 0000000000000000000000000000000000000000000000000000000000000020 // offset of array_1 (1 line down)
            [01] 0000000000000000000000000000000000000000000000000000000000000002 // length of array_1
            [02] 0000000000000000000000000000000000000000000000000000000000000040 // offset of array_2 (2 lines down)
            [03] 00000000000000000000000000000000000000000000000000000000000001e0 // 480 - everything from [04..17]
            [04] 0000000000000000000000000000000000000000000000000000000000000164 // length of array2 356
            [05] 883164560000000000000000c011a73ee8576fb46f5e1c5751ca3b9fe0af2a6f // 32
            [06] 000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 // 64
            [07] 0000000000000000000000000000000000000000000000000000000000002710 // 96
            [08] fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffee530 // 128
            [09] ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff1b18 // 160
            [10] 000000000000000000000000000000000000000000000000016345785d89fd68 // 192
            [11] 00000000000000000000000000000000000000000000000000007f73eca3063a // 224
            [12] 000000000000000000000000000000000000000000000000016042b530ddaec6 // 256
            [13] 00000000000000000000000000000000000000000000000000007e59f044bada // 288
            [14] 000000000000000000000000f847e9d51989033b691b8be943f8e9e268f99b9e // 320
            [15] 0000000000000000000000000000000000000000000000000000000063773477 // 352
            [16] 0000000000000000000000000000000000000000000000000000000000000000 // indication of next
            [17] 0000000000000000000000000000000000000000000000000000000000000004 // length of 4 (next function)
            [18] 12210e8a00000000000000000000000000000000000000000000000000000000 //
    */
    #[test]
    // #[ignore]
    fn test_parse_multicall_2_step() {
        let calldata = "0xac9650d800000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000001e0000000000000000000000000000000000000000000000000000000000000016488316456000000000000000000000000c011a73ee8576fb46f5e1c5751ca3b9fe0af2a6f000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000000000000000000000000000000000000000002710fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffee530ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff1b18000000000000000000000000000000000000000000000000016345785d89fd6800000000000000000000000000000000000000000000000000007f73eca3063a000000000000000000000000000000000000000000000000016042b530ddaec600000000000000000000000000000000000000000000000000007e59f044bada000000000000000000000000f847e9d51989033b691b8be943f8e9e268f99b9e000000000000000000000000000000000000000000000000000000006377347700000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000412210e8a00000000000000000000000000000000000000000000000000000000";
        println!(
            "\nCalldata char len: {}\nBytes: {}",
            calldata.len(),
            calldata.len() / 64 * 32
        );
        let calldata = Calldata::new(calldata);
        calldata.print();
    }

    /*
        Convert the calldata [0]'s etherscan decoded to the following [1]:
        Tx: https://etherscan.io/tx/0x31a45e8893f0cc7de009da5546539f703ed725d076ccdf73d307df5caa8c72b3
        Calldata: 0xac9650d8000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000030000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000012000000000000000000000000000000000000000000000000000000000000002c0000000000000000000000000000000000000000000000000000000000000008413ead56200000000000000000000000061fe7a5257b963f231e1ef6e22cb3b4c6e28c531000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000000000000000000000000000000000000000002710000000000000000000000000000000000000000000831162ce86bc88052f80fd0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001648831645600000000000000000000000061fe7a5257b963f231e1ef6e22cb3b4c6e28c531000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000000000000000000000000000000000000000002710fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffaf178000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002e3bdc25349196582d720000000000000000000000000000000000000000000000000c249fdd32778000000000000000000000000000000000000000000000002e1e525c2ef9dcec50c53000000000000000000000000000000000000000000000000c1cd7c9adfb0d9dc000000000000000000000000ed6c2cb9bf89a2d290e59025837454bf1f144c5000000000000000000000000000000000000000000000000000000000635ce8bf00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000412210e8a00000000000000000000000000000000000000000000000000000000

        Function: multicall(bytes[] data)
        [0] MethodID: 0xac9650d8
            [0]:  0000000000000000000000000000000000000000000000000000000000000020
            [1]:  0000000000000000000000000000000000000000000000000000000000000003
            [2]:  0000000000000000000000000000000000000000000000000000000000000060
            [3]:  0000000000000000000000000000000000000000000000000000000000000120
            [4]:  00000000000000000000000000000000000000000000000000000000000002c0
            [5]:  0000000000000000000000000000000000000000000000000000000000000084
            [6]:  13ead56200000000000000000000000061fe7a5257b963f231e1ef6e22cb3b4c
            [7]:  6e28c531000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead908
            [8]:  3c756cc200000000000000000000000000000000000000000000000000000000
            [9]:  00002710000000000000000000000000000000000000000000831162ce86bc88
            [10]: 052f80fd00000000000000000000000000000000000000000000000000000000
            [11]: 0000000000000000000000000000000000000000000000000000000000000164
            [12]: 8831645600000000000000000000000061fe7a5257b963f231e1ef6e22cb3b4c
            [13]: 6e28c531000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead908
            [14]: 3c756cc200000000000000000000000000000000000000000000000000000000
            [15]: 00002710ffffffffffffffffffffffffffffffffffffffffffffffffffffffff
            [16]: fffaf17800000000000000000000000000000000000000000000000000000000
            [17]: 0000000000000000000000000000000000000000000000000002e3bdc2534919
            [18]: 6582d720000000000000000000000000000000000000000000000000c249fdd3
            [19]: 2778000000000000000000000000000000000000000000000002e1e525c2ef9d
            [20]: cec50c53000000000000000000000000000000000000000000000000c1cd7c9a
            [21]: dfb0d9dc000000000000000000000000ed6c2cb9bf89a2d290e59025837454bf
            [22]: 1f144c5000000000000000000000000000000000000000000000000000000000
            [23]: 635ce8bf00000000000000000000000000000000000000000000000000000000
            [24]: 0000000000000000000000000000000000000000000000000000000000000004
            [25]: 12210e8a00000000000000000000000000000000000000000000000000000000

        [1] MethodID: 0xac9650d8
            0000000000000000000000000000000000000000000000000000000000000020 // offset array_1
            0000000000000000000000000000000000000000000000000000000000000003 // length array_1
            0000000000000000000000000000000000000000000000000000000000000060 // offset array_1A (96/32=3)
            0000000000000000000000000000000000000000000000000000000000000120 // offset array_1B (288/32=9)
            00000000000000000000000000000000000000000000000000000000000002c0 // offset array_1C (704/32=22)
            0000000000000000000000000000000000000000000000000000000000000084 // length array_1A (132 (inc. selector))
            13ead562
            00000000000000000000000061fe7a5257b963f231e1ef6e22cb3b4c6e28c531 // 32
            000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 // 64
            0000000000000000000000000000000000000000000000000000000000002710 // 96
            000000000000000000000000000000000000000000831162ce86bc88052f80fd // 128 (+ selector (4) = 132)
            0000000000000000000000000000000000000000000000000000000000000000 // indication of next
            00000000000000000000000000000000000000000000000000000164         // length of 356 (next function)
            88316456
            00000000000000000000000061fe7a5257b963f231e1ef6e22cb3b4c6e28c531 // 32
            000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2 // 64
            0000000000000000000000000000000000000000000000000000000000002710 // 96
            fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffaf178 // 128
            0000000000000000000000000000000000000000000000000000000000000000 // 160
            00000000000000000000000000000000000000000002e3bdc25349196582d720 // 192
            000000000000000000000000000000000000000000000000c249fdd327780000 // 224
            00000000000000000000000000000000000000000002e1e525c2ef9dcec50c53 // 256
            000000000000000000000000000000000000000000000000c1cd7c9adfb0d9dc // 288
            000000000000000000000000ed6c2cb9bf89a2d290e59025837454bf1f144c50 // 320
            00000000000000000000000000000000000000000000000000000000635ce8bf // 352 (+ selector = 356)
            0000000000000000000000000000000000000000000000000000000000000000 // indication of next
            00000000000000000000000000000000000000000000000000000004         // length of 4 (next function)
            12210e8a00000000000000000000000000000000000000000000000000000000 // 4
    */
    #[test]
    #[ignore]
    fn test_parse_multicall_3_step() {
        let calldata = "0xac9650d8000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000030000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000012000000000000000000000000000000000000000000000000000000000000002c0000000000000000000000000000000000000000000000000000000000000008413ead56200000000000000000000000061fe7a5257b963f231e1ef6e22cb3b4c6e28c531000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000000000000000000000000000000000000000002710000000000000000000000000000000000000000000831162ce86bc88052f80fd0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001648831645600000000000000000000000061fe7a5257b963f231e1ef6e22cb3b4c6e28c531000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000000000000000000000000000000000000000002710fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffaf178000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002e3bdc25349196582d720000000000000000000000000000000000000000000000000c249fdd32778000000000000000000000000000000000000000000000002e1e525c2ef9dcec50c53000000000000000000000000000000000000000000000000c1cd7c9adfb0d9dc000000000000000000000000ed6c2cb9bf89a2d290e59025837454bf1f144c5000000000000000000000000000000000000000000000000000000000635ce8bf00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000412210e8a00000000000000000000000000000000000000000000000000000000";
        println!(
            "\nCalldata char len: {}\nBytes: {}",
            calldata.len(),
            calldata.len() / 64 * 32
        );
        let calldata = Calldata::new(calldata);
        calldata.print();
    }

    /*
        Tx: https://testnet.ftmscan.com/tx/0xa0801171ed2811082946ff7ff57e9470f98dcf5e64254bafd1d08ca959a051b7
        Calldata: 0xcf97008600000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000001800000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000003313233000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000023435000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000436313334000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000e0000000000000000000000000000000000000000000000000000000000000000161000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001620000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000016300000000000000000000000000000000000000000000000000000000000000
        Function: embed(string[][] strings)
        MethodID: 0xcf970086
        [0]:  0000000000000000000000000000000000000000000000000000000000000020
        [1]:  0000000000000000000000000000000000000000000000000000000000000002
        [2]:  0000000000000000000000000000000000000000000000000000000000000040
        [3]:  0000000000000000000000000000000000000000000000000000000000000180
        [4]:  0000000000000000000000000000000000000000000000000000000000000003
        [5]:  0000000000000000000000000000000000000000000000000000000000000060
        [6]:  00000000000000000000000000000000000000000000000000000000000000a0
        [7]:  00000000000000000000000000000000000000000000000000000000000000e0
        [8]:  0000000000000000000000000000000000000000000000000000000000000003
        [9]:  3132330000000000000000000000000000000000000000000000000000000000
        [10]: 0000000000000000000000000000000000000000000000000000000000000002
        [11]: 3435000000000000000000000000000000000000000000000000000000000000
        [12]: 0000000000000000000000000000000000000000000000000000000000000004
        [13]: 3631333400000000000000000000000000000000000000000000000000000000
        [14]: 0000000000000000000000000000000000000000000000000000000000000003
        [15]: 0000000000000000000000000000000000000000000000000000000000000060
        [16]: 00000000000000000000000000000000000000000000000000000000000000a0
        [17]: 00000000000000000000000000000000000000000000000000000000000000e0
        [18]: 0000000000000000000000000000000000000000000000000000000000000001
        [19]: 6100000000000000000000000000000000000000000000000000000000000000
        [20]: 0000000000000000000000000000000000000000000000000000000000000001
        [21]: 6200000000000000000000000000000000000000000000000000000000000000
        [22]: 0000000000000000000000000000000000000000000000000000000000000001
        [23]: 6300000000000000000000000000000000000000000000000000000000000000
        TODO...UNFINISHED TEST
    */
    #[test]
    #[ignore]
    fn test_parse_nested_strings() {
        let calldata = "0xcf97008600000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000001800000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000e00000000000000000000000000000000000000000000000000000000000000003313233000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000023435000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000436313334000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000e0000000000000000000000000000000000000000000000000000000000000000161000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001620000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000016300000000000000000000000000000000000000000000000000000000000000";
        println!(
            "\nCalldata char len: {}\nBytes: {}",
            calldata.len(),
            calldata.len() / 64 * 32
        );
        let calldata = Calldata::new(calldata);
        calldata.print();
    }

    // Function: multicall(uint256 deadline,bytes[] data)
    // MethodID: 0x5ae401dc
    // [0]:  00000000000000000000000000000000000000000000000000000000638292b3
    // [1]:  0000000000000000000000000000000000000000000000000000000000000040
    // [2]:  0000000000000000000000000000000000000000000000000000000000000002
    // [3]:  0000000000000000000000000000000000000000000000000000000000000040
    // [4]:  0000000000000000000000000000000000000000000000000000000000000140
    // [5]:  00000000000000000000000000000000000000000000000000000000000000c4
    // [6]:  4659a4940000000000000000000000006b175474e89094c44da98b954eedeac4
    // [7]:  95271d0f00000000000000000000000000000000000000000000000000000000
    // [8]:  0000000100000000000000000000000000000000000000000000000000000000 --
    // [9]:  638296c700000000000000000000000000000000000000000000000000000000 --
    // [10]: 0000001c8892b2afb729fb079b7786393f3884f1d7317f18e9692bf4e8db90cf --
    // [11]: 97f5854967048010f45d896e0c465dad3952be95afce410d0769c4014c827c20 --
    // [12]: f0cc525d00000000000000000000000000000000000000000000000000000000 --
    // [13]: 00000000000000000000000000000000000000000000000000000000000000e4 --
    // [14]: 04e45aaf0000000000000000000000006b175474e89094c44da98b954eedeac4 --
    // [15]: 95271d0f000000000000000000000000a0b86991c6218b36c1d19d4a2e9eb0ce --
    // [16]: 3606eb4800000000000000000000000000000000000000000000000000000000 --
    // [17]: 000001f4000000000000000000000000a9af48f8cd3df47f913eefb032386f2d --
    // [18]: 6debfb3500000000000000000000000000000000000000000000001be7653538 --
    // [19]: b68d564a00000000000000000000000000000000000000000000000000000000
    // [20]: 1e8297ae00000000000000000000000000000000000000000000000000000000
    // [21]: 0000000000000000000000000000000000000000000000000000000000000000

    /// MethodID: 0x5ae401dc
    /// 00000000000000000000000000000000000000000000000000000000638292b3 // uint256 1669501619
    /// 0000000000000000000000000000000000000000000000000000000000000040 // offset array_1
    /// 0000000000000000000000000000000000000000000000000000000000000002 // length array_1
    /// 0000000000000000000000000000000000000000000000000000000000000040 // offset array_1A
    /// 0000000000000000000000000000000000000000000000000000000000000140 // offset array_1B
    /// 00000000000000000000000000000000000000000000000000000000000000c4 // length array_1A (196-4/32=6)
    /// 4659a494
    /// 0000000000000000000000006b175474e89094c44da98b954eedeac495271d0f // 32
    /// 0000000000000000000000000000000000000000000000000000000000000001 // 64
    /// 00000000000000000000000000000000000000000000000000000000638296c7 // 96
    /// 000000000000000000000000000000000000000000000000000000000000001c // 128
    /// 8892b2afb729fb079b7786393f3884f1d7317f18e9692bf4e8db90cf97f58549 // 160
    /// 67048010f45d896e0c465dad3952be95afce410d0769c4014c827c20f0cc525d // 192
    /// 0000000000000000000000000000000000000000000000000000000000000000
    /// 00000000000000000000000000000000000000000000000000000000000000e4 // length array_1B (228-4/32=)
    /// 3606eb48
    /// 0000000000000000000000006b175474e89094c44da98b954eedeac495271d0f // 32
    /// 000000000000000000000000a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48 // 64
    /// 00000000000000000000000000000000000000000000000000000000000001f4 // 96
    /// 000000000000000000000000a9af48f8cd3df47f913eefb032386f2d6debfb35 // 128
    /// 00000000000000000000000000000000000000000000001be7653538b68d564a // 160
    /// 000000000000000000000000000000000000000000000000000000001e8297ae // 192
    /// 0000000000000000000000000000000000000000000000000000000000000000 // 224

    /// TODO...UNFINISHED TEST
    /// https://etherscan.io/tx/0x1fb87cad877c5335bb1c756ae6ed338eb08e0acc9a086880967d4323537a1416
    #[test]
    #[ignore]
    fn test_uniswap_v3_router_2() {
        let calldata = "0x5ae401dc00000000000000000000000000000000000000000000000000000000638292b3000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000014000000000000000000000000000000000000000000000000000000000000000c44659a4940000000000000000000000006b175474e89094c44da98b954eedeac495271d0f000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000638296c7000000000000000000000000000000000000000000000000000000000000001c8892b2afb729fb079b7786393f3884f1d7317f18e9692bf4e8db90cf97f5854967048010f45d896e0c465dad3952be95afce410d0769c4014c827c20f0cc525d0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000e404e45aaf0000000000000000000000006b175474e89094c44da98b954eedeac495271d0f000000000000000000000000a0b86991c6218b36c1d19d4a2e9eb0ce3606eb4800000000000000000000000000000000000000000000000000000000000001f4000000000000000000000000a9af48f8cd3df47f913eefb032386f2d6debfb3500000000000000000000000000000000000000000000001be7653538b68d564a000000000000000000000000000000000000000000000000000000001e8297ae000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
        println!(
            "\nCalldata char len: {}\nBytes: {}",
            calldata.len(),
            calldata.len() / 64 * 32
        );
        let calldata = Calldata::new(calldata);
        calldata.print();
    }

    /*
    0x
    710a9f68
    00000000000000000000000000000000000000000000000000000000000005e4
    000000000000000000000000dc9c7a2bae15dd89271ae5701a6f4db147baa44c
    0000000000000000000000000000000000000000000000000000000000000060
    0000000000000000000000000000000000000000000000000000000000000124
    95723b1c
    0000000000000000000000006b175474e89094c44da98b954eedeac495271d0f
    000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2
    00000000000000000000000000000000000000000000000211d72bb3
    049586a7
    0000000000000000000000000000000000000000000000000000000000000000
    0000000000000000000000000000000000000000000000000000000000000000
    0000000000000000000000000000000000000000000000000000000000000000
    0000000000000000000000000000000000000000000000000000000000000000
    00000000000000000000000000000000000000000000006ee543b3be5a28a8f9
    00000000000000000000000000000000000000000000000016687535bce57786
    00000000000000000000000000000000000000000000000000000000
    */
    #[test]
    #[ignore]
    fn test_multicall_homora() {
        let calldata = "0x710a9f6800000000000000000000000000000000000000000000000000000000000005e4000000000000000000000000dc9c7a2bae15dd89271ae5701a6f4db147baa44c0000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000012495723b1c0000000000000000000000006b175474e89094c44da98b954eedeac495271d0f000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc200000000000000000000000000000000000000000000000211d72bb3049586a7000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000006ee543b3be5a28a8f900000000000000000000000000000000000000000000000016687535bce5778600000000000000000000000000000000000000000000000000000000";
        println!(
            "\nCalldata char len: {}\nBytes: {}",
            calldata.len(),
            calldata.len() / 64 * 32
        );
        let calldata = Calldata::new(calldata);
        calldata.print();
    }
}
