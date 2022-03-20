use crate::{
    gdt,
    util::{self, out8, in8}, drivers::keyboard::Keyboard,
    processes::{SYSCALL_SP, SYSCALL_UMAP, SYSCALL_USP}
};
use lazy_static::lazy_static;
use x86_64::structures::idt::{self, InterruptDescriptorTable};


use crate::drivers::pic::ChainedPics;
use spin;

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

pub static PICS: spin::Mutex<ChainedPics> =
    spin::Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

#[repr(C)]
pub struct CpuSnapshot {
    pub rbp: u64,

    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,

    pub rdi: u64,
    pub rsi: u64,

    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
}

#[macro_export]
macro_rules! interrupt_begin {
    () => {
        unsafe {
        asm!(
        "
        cli
        push rax
        push rbx
        push rcx
        push rdx

        push rsi
        push rdi

        push r8
        push r9
        push r10
        push r11
        push r12
        push r13
        push r14
        push r15

        push rbp
        ",
        options(nomem, nostack)
    );

    // if crate::processes::SYSCALL_SP != 0 {
    //     asm!("mov rdx, rsp", options(nostack));
    //     asm!("mov rsp, {}", in(reg) crate::processes::SYSCALL_SP, options(nostack));
    //     asm!("mov rsi, cr3", options(nostack));
    //     asm!("mov cr3, {}", in(reg) crate::mem::KERNEL_MAP, options(nostack));
    //     asm!("mov {}, rdx ; push rsi", out(reg) crate::processes::SYSCALL_USP, options(nostack));
    // }

}
    };
}

#[macro_export]
macro_rules! interrupt_end {
    () => {
        unsafe {
            //     if crate::processes::SYSCALL_USP != 0 {
            //     asm!("pop rsi ; mov cr3, rsi", options(nostack));
            //     asm!("mov rsp, {}", in(reg) crate::processes::SYSCALL_USP, options(nostack));
            // }

            asm!(
                "
        pop rbp
        
        pop r15
        pop r14
        pop r13
        pop r12
        pop r11
        pop r10
        pop r9
        pop r8

        
        pop rdi
        pop rsi

        pop rdx
        pop rcx
        pop rbx
        pop rax
        ",
                options(nomem, nostack)
            );
        }
    };
    (sti) => {
        unsafe {
            //     if crate::processes::SYSCALL_USP != 0 {
            //     asm!("pop rsi ; mov cr3, rsi", options(nostack));
            //     asm!("mov rsp, {}", in(reg) crate::processes::SYSCALL_USP, options(nostack));
            // }

            asm!(
                "
        pop rbp
        
        pop r15
        pop r14
        pop r13
        pop r12
        pop r11
        pop r10
        pop r9
        pop r8

        
        pop rdi
        pop rsi

        pop rdx
        pop rcx
        pop rbx
        pop rax
        sti
        ",
                options(nomem, nostack)
            );
        }
    };
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
    Cascade,
    SerialPort1,
    SerialPort2,
    ParallelPort1,
    FloppyDisk,
    ParallelPort2,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }

    fn as_usize(self) -> usize {
        usize::from(self.as_u8())
    }
}

lazy_static! {
    pub static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.general_protection_fault
            .set_handler_fn(general_protection_handler);
        idt.non_maskable_interrupt.set_handler_fn(nmi_handler);
        idt.segment_not_present
            .set_handler_fn(segment_not_present_handler);

        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
            idt.page_fault.set_handler_fn(pagefault_handler).set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }

        unsafe {
            idt[InterruptIndex::Timer.as_usize()]
                .set_handler_fn(timer_handler_stub)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
            idt[InterruptIndex::Keyboard.as_usize()]
                .set_handler_fn(keyboard_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt[InterruptIndex::SerialPort2.as_usize()].set_handler_fn(serial1_handler);
        idt[InterruptIndex::SerialPort1.as_usize()].set_handler_fn(serial1_handler);
        idt
    };
}

pub fn init() {
    x86_64::instructions::interrupts::disable();
    IDT.load();

    unsafe {
        PICS.lock().initialize();
    };

    // The above initialize method doesn't do this and doesn't work without it :(
    unsafe {
        out8(0x21, 0x00);
        out8(0xA1, 0x00);
    }
    // unsafe { PICS.lock().write_masks(0xfe, 0xff) }

}

extern "x86-interrupt" fn timer_handler_stub(stack_frame: idt::InterruptStackFrame) {
    // x86_64::instructions::hlt();
    interrupt_begin!();
    unsafe {
        asm!("mov rdx, rsp", options(nomem, nostack)); // Save old stack
        asm!("mov rsp, rax; mov rbp, rsp", in("rax") SYSCALL_SP, options(nostack)); // Load kernel stack
        asm!("", out("rdx") SYSCALL_USP, options(nostack)); // Save old stack into variable for later
        // asm!("", out("rdx") cpu, options(nostack)); // Load address into pointer for cpu snapshot

        asm!("mov rax, cr3", out("rax") SYSCALL_UMAP, options(nostack)); // Save old address space

        timer_handler();

        asm!("mov cr3, rax", in("rax") SYSCALL_UMAP, options(nostack));
        asm!("mov rsp, rax", in("rax") SYSCALL_USP, options(nostack));

        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
        // PICS.force_unlock();
    }
    interrupt_end!(sti);
}

fn timer_handler() {

}

extern "x86-interrupt" fn serial1_handler(_stack_frame: idt::InterruptStackFrame) {
    kprintln!("Serial\n");
    unsafe {
        util::in8(0x3F8);
    }
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::SerialPort2.as_u8());
    }
}

extern "x86-interrupt" fn keyboard_handler(_stack_frame: idt::InterruptStackFrame) {
    interrupt_begin!();
     unsafe {
        asm!("mov rdx, rsp", options(nostack));
        asm!("mov rsp, {}", in(reg) crate::processes::SYSCALL_SP, options(nostack));

        asm!("mov rsi, cr3", options(nostack));
        // asm!("mov cr3, {}", in(reg) crate::mem::KERNEL_MAP, options(nostack));
        asm!("mov {}, rdx", out(reg) crate::processes::SYSCALL_USP, options(nostack));

        asm!("push rsi", options(nostack));

        let b = in8(0x60);
        let chr = Keyboard::code_to_char(b);
        kprint!("{}", chr);

        asm!("pop rsi", options(nostack));
        asm!("mov cr3, rsi", options(nostack));
        asm!("mov rsp, {}", in(reg) crate::processes::SYSCALL_USP, options(nostack));

        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }
    interrupt_end!();
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: idt::InterruptStackFrame) {
    kprintln!("EXCPETION: BREAKPOINT\n{:#?}\n", stack_frame);
}

extern "x86-interrupt" fn nmi_handler(stack_frame: idt::InterruptStackFrame) {
    kprintln!("EXCPETION: NMI\n{:#?}\n", stack_frame);
}

extern "x86-interrupt" fn segment_not_present_handler(
    stack_frame: idt::InterruptStackFrame,
    e: u64,
) {
}

extern "x86-interrupt" fn general_protection_handler(
    stack_frame: idt::InterruptStackFrame,
    _e: u64,
) {
    kprintln!("EXCPETION: GP\n{:#?}\n{}\n", stack_frame, _e);
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: idt::InterruptStackFrame,
    _e: u64,
) -> ! {
    kprintln!("EXCPETION: Double Fault\n{:#?}\n{}\n", stack_frame, _e);
    loop {}
}

extern "x86-interrupt" fn pagefault_handler(
    _stack_frame: idt::InterruptStackFrame,
    _error_code: idt::PageFaultErrorCode,
) {
    use x86_64::registers::control::Cr2;

    kprintln!(
        "EXCPETION: PAGE FAULT\n{:#?}\n{:#?}\n",
        _stack_frame,
        _error_code
    );
    kprintln!("Address: {:?}\n", Cr2::read());

    loop {}
}
