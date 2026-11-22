//! Opt-in native interpreter handoff using the initrd test overlay.
use super::*;
use crate::process;

const MAIN_PATH: &str = "/tests/elf-handoff/main";
const INTERPRETER_PATH: &str = "/tests/elf-handoff/interpreter";
const INTERPRETER_BIAS: usize = 0x2000_0000;
const MAIN_EXIT_STATUS: i32 = 37;

pub(crate) fn run_handoff_smoke() {
    let Some(initrd) = crate::initrd::get_initrd() else {
        return;
    };
    let Some(main) = initrd.entries().iter().find(|entry| entry.name == MAIN_PATH) else {
        return;
    };
    let ok = initrd
        .entries()
        .iter()
        .find(|entry| entry.name == INTERPRETER_PATH)
        .is_some_and(|interpreter| run(main.data, interpreter.data));
    crate::serial_write(if ok {
        "[ELF-HANDOFF] interpreter auxv and main entry OK\r\n"
    } else {
        "[ELF-HANDOFF] FAIL\r\n"
    });
}

fn run(main: &[u8], interpreter: &[u8]) -> bool {
    let Ok(parser) = ElfParser::parse_image(main) else {
        return false;
    };
    let Ok((main, interpreter)) = parser.interpreted_load_plan(
        INTERPRETER_PATH,
        interpreter,
        0,
        INTERPRETER_BIAS,
        PAGE_SIZE..USER_STACK_TOP - USER_STACK_SIZE,
    ) else {
        return false;
    };
    let Ok(pid) = process::create_user_process_record(String::from(MAIN_PATH), true, false) else {
        return false;
    };
    let ok = prepare_process_image(
        pid,
        &parser,
        &main,
        Some(&interpreter),
        &[String::from(MAIN_PATH)],
        &[],
        true,
    )
    .is_ok()
        && process::enter_user_process(pid).is_ok_and(|exit| {
            exit.status == process::ProcessExitStatus::Exited(MAIN_EXIT_STATUS)
        });
    process::autoreap_process(pid, "elf-handoff-smoke").is_ok() && ok
}
