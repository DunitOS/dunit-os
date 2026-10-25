#![no_std]
#![no_main]

extern crate alloc;

#[dunit::main]
fn main() -> dunit::Result<()> {
    dunit::println!("calc: enter expressions like 2+2 or 7 * 3")?;
    dunit::println!("calc: type exit to quit, Ctrl+C interrupts while waiting")?;
    let mut input = dunit::io::BufReader::new(dunit::io::stdin());
    loop {
        dunit::print!("calc> ")?;
        let Some(line) = input.read_line()? else {
            return Ok(());
        };
        let expression = line.trim();
        if expression == "exit" || expression == "quit" {
            dunit::println!("calc: bye")?;
            return Ok(());
        }
        match eval(expression) {
            Ok(value) => dunit::println!("{}", value)?,
            Err(message) => dunit::println!("{}", message)?,
        }
    }
}

fn eval(input: &str) -> Result<i64, &'static str> {
    let bytes = input.as_bytes();
    let mut index = 0usize;
    skip_spaces(bytes, &mut index);
    let left = parse_i64(bytes, &mut index)?;
    skip_spaces(bytes, &mut index);
    if index >= bytes.len() {
        return Ok(left);
    }
    let op = bytes[index];
    index += 1;
    skip_spaces(bytes, &mut index);
    let right = parse_i64(bytes, &mut index)?;
    skip_spaces(bytes, &mut index);
    if index != bytes.len() {
        return Err("calc: expected end of input");
    }

    match op {
        b'+' => Ok(left + right),
        b'-' => Ok(left - right),
        b'*' => Ok(left * right),
        b'/' => {
            if right == 0 {
                Err("calc: division by zero")
            } else {
                Ok(left / right)
            }
        }
        _ => Err("calc: expected operator + - * /"),
    }
}

fn parse_i64(bytes: &[u8], index: &mut usize) -> Result<i64, &'static str> {
    let mut sign = 1i64;
    if *index < bytes.len() && bytes[*index] == b'-' {
        sign = -1;
        *index += 1;
    }
    if *index >= bytes.len() || !bytes[*index].is_ascii_digit() {
        return Err("calc: expected number");
    }

    let mut value = 0i64;
    while *index < bytes.len() && bytes[*index].is_ascii_digit() {
        value = value * 10 + (bytes[*index] - b'0') as i64;
        *index += 1;
    }
    Ok(value * sign)
}

fn skip_spaces(bytes: &[u8], index: &mut usize) {
    while *index < bytes.len() && bytes[*index] == b' ' {
        *index += 1;
    }
}
