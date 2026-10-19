use std::process::{Child, Command};
use std::time::{Duration, Instant};

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command};

    pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    const KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const BASIC_ACCOUNTING: i32 = 1;
    const EXTENDED_LIMITS: i32 = 9;

    #[repr(C)]
    #[derive(Default)]
    struct BasicLimits {
        per_process_user_time: i64,
        per_job_user_time: i64,
        flags: u32,
        min_working_set: usize,
        max_working_set: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io: [u64; 6],
        process_memory: usize,
        job_memory: usize,
        peak_process_memory: usize,
        peak_job_memory: usize,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Accounting {
        user_time: i64,
        kernel_time: i64,
        period_user_time: i64,
        period_kernel_time: i64,
        page_faults: u32,
        total_processes: u32,
        active_processes: u32,
        terminated_processes: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *mut c_void, name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(job: *mut c_void, class: i32, info: *const c_void, length: u32) -> i32;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn TerminateJobObject(job: *mut c_void, exit_code: u32) -> i32;
        fn QueryInformationJobObject(job: *mut c_void, class: i32, info: *mut c_void, length: u32, returned: *mut u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn GetLastError() -> u32;
        fn GetStdHandle(which: u32) -> *mut c_void;
        fn SetHandleInformation(handle: *mut c_void, mask: u32, flags: u32) -> i32;
    }

    const STD_INPUT: u32 = -10i32 as u32;
    const STD_OUTPUT: u32 = -11i32 as u32;
    const STD_ERROR: u32 = -12i32 as u32;
    const HANDLE_FLAG_INHERIT: u32 = 1;

    pub struct Job(*mut c_void);

    impl Job {
        pub fn new() -> Result<Job, String> {
            let handle = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
            if handle.is_null() {
                return Err(format!("CreateJobObjectW failed: error {}", unsafe { GetLastError() }));
            }
            let job = Job(handle);
            let mut limits = ExtendedLimits::default();
            limits.basic.flags = KILL_ON_JOB_CLOSE;
            let size = std::mem::size_of::<ExtendedLimits>() as u32;
            if unsafe { SetInformationJobObject(job.0, EXTENDED_LIMITS, &limits as *const _ as *const c_void, size) } == 0 {
                return Err(format!("SetInformationJobObject failed: error {}", unsafe { GetLastError() }));
            }
            Ok(job)
        }

        pub fn prepare(&self, command: &mut Command) {
            command.creation_flags(CREATE_NO_WINDOW);
        }

        pub fn assign(&mut self, child: &Child) -> Result<(), String> {
            if unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle()) } == 0 {
                return Err(format!("AssignProcessToJobObject failed: error {}", unsafe { GetLastError() }));
            }
            Ok(())
        }

        pub fn terminate(&self) {
            unsafe { TerminateJobObject(self.0, 1) };
        }

        pub fn active(&self) -> Option<u32> {
            let mut info = Accounting::default();
            let size = std::mem::size_of::<Accounting>() as u32;
            let ok = unsafe { QueryInformationJobObject(self.0, BASIC_ACCOUNTING, &mut info as *mut _ as *mut c_void, size, std::ptr::null_mut()) };
            (ok != 0).then_some(info.active_processes)
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    pub fn spawn_detached(command: &mut Command) -> std::io::Result<Child> {
        for which in [STD_INPUT, STD_OUTPUT, STD_ERROR] {
            let handle = unsafe { GetStdHandle(which) };
            if !handle.is_null() && handle as isize != -1 {
                unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
            }
        }
        let base = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
        command.creation_flags(base | CREATE_BREAKAWAY_FROM_JOB);
        match command.spawn() {
            Ok(child) => Ok(child),
            Err(_) => {
                command.creation_flags(base);
                command.spawn()
            }
        }
    }

    pub fn hide_window(command: &mut Command) {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

#[cfg(not(windows))]
mod sys {
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};

    pub struct Job(Option<u32>);

    impl Job {
        pub fn new() -> Result<Job, String> {
            Ok(Job(None))
        }

        pub fn prepare(&self, command: &mut Command) {
            command.process_group(0);
        }

        pub fn assign(&mut self, child: &Child) -> Result<(), String> {
            self.0 = Some(child.id());
            Ok(())
        }

        pub fn terminate(&self) {
            if let Some(group) = self.0 {
                let _ = Command::new("kill").args(["-KILL", "--", &format!("-{group}")]).status();
            }
        }

        pub fn active(&self) -> Option<u32> {
            None
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            self.terminate();
        }
    }

    pub fn spawn_detached(command: &mut Command) -> std::io::Result<Child> {
        command.process_group(0);
        command.spawn()
    }

    pub fn hide_window(_: &mut Command) {}
}

pub struct JobGuard(sys::Job);

impl JobGuard {
    pub fn new() -> Result<JobGuard, String> {
        sys::Job::new().map(JobGuard)
    }

    pub fn prepare(&self, command: &mut Command) {
        self.0.prepare(command);
    }

    pub fn assign(&mut self, child: &Child) -> Result<(), String> {
        self.0.assign(child)
    }

    pub fn terminate(&self) {
        self.0.terminate();
    }

    pub fn wait_empty(&self, timeout: Duration) -> Option<u32> {
        let deadline = Instant::now() + timeout;
        loop {
            let active = self.0.active()?;
            if active == 0 || Instant::now() >= deadline {
                return Some(active);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

pub fn spawn_detached(command: &mut Command) -> std::io::Result<Child> {
    sys::spawn_detached(command)
}

pub fn hide_window(command: &mut Command) {
    sys::hide_window(command);
}
