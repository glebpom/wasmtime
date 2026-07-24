use cranelift_codegen::ir::*;
use cranelift_codegen::isa::{CallConv, OwnedTargetIsa};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::{Context, ir::types::I16};
use cranelift_entity::EntityRef;
use cranelift_frontend::*;
use cranelift_jit::*;
use cranelift_module::*;

fn isa() -> Option<OwnedTargetIsa> {
    let mut flag_builder = settings::builder();
    flag_builder.set("use_colocated_libcalls", "false").unwrap();
    // FIXME set back to true once the x64 backend supports it.
    flag_builder.set("is_pic", "false").unwrap();
    let isa_builder = cranelift_native::builder().ok()?;
    isa_builder.finish(settings::Flags::new(flag_builder)).ok()
}

#[test]
fn error_on_incompatible_sig_in_declare_function() {
    let Some(isa) = isa() else {
        return;
    };
    let mut module = JITModule::new(JITBuilder::with_isa(isa, default_libcall_names()));

    let mut sig = Signature {
        params: vec![AbiParam::new(types::I64)],
        returns: vec![],
        call_conv: CallConv::SystemV,
    };
    module
        .declare_function("abc", Linkage::Local, &sig)
        .unwrap();
    sig.params[0] = AbiParam::new(types::I32);
    module
        .declare_function("abc", Linkage::Local, &sig)
        .err()
        .unwrap(); // Make sure this is an error
}

fn define_simple_function(module: &mut JITModule) -> Result<FuncId, ModuleError> {
    let sig = Signature {
        params: vec![],
        returns: vec![],
        call_conv: CallConv::SystemV,
    };

    let func_id = module.declare_function("abc", Linkage::Local, &sig)?;

    let mut ctx = Context::new();
    ctx.func = Function::with_name_signature(UserFuncName::user(0, func_id.as_u32()), sig);
    let mut func_ctx = FunctionBuilderContext::new();
    {
        let mut bcx: FunctionBuilder = FunctionBuilder::new(&mut ctx.func, &mut func_ctx);
        let block = bcx.create_block();
        bcx.switch_to_block(block);
        bcx.ins().return_(&[]);
    }

    module.define_function(func_id, &mut ctx)?;

    Ok(func_id)
}

#[test]
fn panic_on_define_after_finalize() {
    let Some(isa) = isa() else {
        return;
    };
    let mut module = JITModule::new(JITBuilder::with_isa(isa, default_libcall_names()));

    define_simple_function(&mut module).unwrap();
    define_simple_function(&mut module).err().unwrap();
}

#[test]
fn switch_error() {
    use cranelift_codegen::settings;

    let sig = Signature {
        params: vec![AbiParam::new(types::I32)],
        returns: vec![AbiParam::new(types::I32)],
        call_conv: CallConv::SystemV,
    };

    let mut func = Function::with_name_signature(UserFuncName::default(), sig);

    let mut func_ctx = FunctionBuilderContext::new();
    {
        let mut bcx: FunctionBuilder = FunctionBuilder::new(&mut func, &mut func_ctx);
        let start = bcx.create_block();
        let bb0 = bcx.create_block();
        let bb1 = bcx.create_block();
        let bb2 = bcx.create_block();
        let bb3 = bcx.create_block();
        println!("{start} {bb0} {bb1} {bb2} {bb3}");

        bcx.declare_var(types::I32);
        bcx.declare_var(types::I32);
        let in_val = bcx.append_block_param(start, types::I32);
        bcx.switch_to_block(start);
        bcx.def_var(Variable::new(0), in_val);
        bcx.ins().jump(bb0, &[]);

        bcx.switch_to_block(bb0);
        let discr = bcx.use_var(Variable::new(0));
        let mut switch = cranelift_frontend::Switch::new();
        for &(index, bb) in &[
            (9, bb1),
            (13, bb1),
            (10, bb1),
            (92, bb1),
            (39, bb1),
            (34, bb1),
        ] {
            switch.set_entry(index, bb);
        }
        switch.emit(&mut bcx, discr, bb2);

        bcx.switch_to_block(bb1);
        let v = bcx.use_var(Variable::new(0));
        bcx.def_var(Variable::new(1), v);
        bcx.ins().jump(bb3, &[]);

        bcx.switch_to_block(bb2);
        let v = bcx.use_var(Variable::new(0));
        bcx.def_var(Variable::new(1), v);
        bcx.ins().jump(bb3, &[]);

        bcx.switch_to_block(bb3);
        let r = bcx.use_var(Variable::new(1));
        bcx.ins().return_(&[r]);

        bcx.seal_all_blocks();
        bcx.finalize();
    }

    let flags = settings::Flags::new(settings::builder());
    match cranelift_codegen::verify_function(&func, &flags) {
        Ok(_) => {}
        Err(err) => {
            let pretty_error =
                cranelift_codegen::print_errors::pretty_verifier_error(&func, None, err);
            panic!("pretty_error:\n{pretty_error}");
        }
    }
}

#[test]
fn libcall_function() {
    let Some(isa) = isa() else {
        return;
    };
    let mut module = JITModule::new(JITBuilder::with_isa(isa, default_libcall_names()));

    let sig = Signature {
        params: vec![],
        returns: vec![],
        call_conv: CallConv::SystemV,
    };

    let func_id = module
        .declare_function("function", Linkage::Local, &sig)
        .unwrap();

    let mut ctx = Context::new();
    ctx.func = Function::with_name_signature(UserFuncName::user(0, func_id.as_u32()), sig);

    let mut func_ctx = FunctionBuilderContext::new();
    {
        let mut bcx: FunctionBuilder = FunctionBuilder::new(&mut ctx.func, &mut func_ctx);
        let block = bcx.create_block();
        bcx.switch_to_block(block);

        let int = module.target_config().pointer_type();
        let zero = bcx.ins().iconst(I16, 0);
        let size = bcx.ins().iconst(int, 10);

        let mut signature = module.make_signature();
        signature.params.push(AbiParam::new(int));
        signature.returns.push(AbiParam::new(int));
        let callee = module
            .declare_function("malloc", Linkage::Import, &signature)
            .expect("declare malloc function");
        let local_callee = module.declare_func_in_func(callee, &mut bcx.func);
        let argument_exprs = vec![size];
        let call = bcx.ins().call(local_callee, &argument_exprs);
        let buffer = bcx.inst_results(call)[0];

        bcx.call_memset(module.target_config(), buffer, zero, size);

        bcx.ins().return_(&[]);
    }

    module
        .define_function_with_control_plane(func_id, &mut ctx, &mut Default::default())
        .unwrap();

    module.finalize_definitions().unwrap();
}

// This used to cause UB. See https://github.com/bytecodealliance/wasmtime/issues/7918.
#[test]
fn empty_data_object() {
    let Some(isa) = isa() else {
        return;
    };
    let mut module = JITModule::new(JITBuilder::with_isa(isa, default_libcall_names()));

    let data_id = module
        .declare_data("empty", Linkage::Export, false, false)
        .unwrap();

    let mut data = DataDescription::new();
    data.define(Box::new([]));
    module.define_data(data_id, &data).unwrap();
}

/// Reproduces a bug where a `call` between two functions of the same module
/// that happen to be placed further than ±2 GiB apart panicked while applying
/// the `X86CallPCRel4` relocation instead of routing the call through a
/// veneer.
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
#[test]
fn far_x86_call_uses_veneer() {
    use std::alloc::{Layout, alloc_zeroed, dealloc};
    use std::io;
    use std::mem;
    use std::ptr;

    use cranelift_codegen::binemit::Reloc;

    /// A large (a bit more than 4 GiB) virtual memory reservation that
    /// executable allocations are carved out of, so that two functions can
    /// deterministically be placed out of `rel32` range of each other.
    struct ReservedAddressSpace {
        base: usize,
        len: usize,
        page_size: usize,
    }

    impl ReservedAddressSpace {
        fn new() -> Self {
            let page_size = usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap();
            let len = (i32::MAX as usize)
                .checked_mul(2)
                .and_then(|size| size.checked_add(16 * 1024 * 1024))
                .unwrap()
                .next_multiple_of(page_size);
            let mapping = unsafe {
                libc::mmap(
                    ptr::null_mut(),
                    len,
                    libc::PROT_NONE,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE,
                    -1,
                    0,
                )
            };
            assert_ne!(mapping, libc::MAP_FAILED);
            Self {
                base: mapping.addr(),
                len,
                page_size,
            }
        }
    }

    impl Drop for ReservedAddressSpace {
        fn drop(&mut self) {
            let result = unsafe { libc::munmap(self.base as *mut libc::c_void, self.len) };
            assert_eq!(result, 0);
        }
    }

    /// A memory provider which places the first executable allocation at the
    /// very bottom of the reserved address space and the second one at the
    /// very top, more than ±2^31 bytes away from the first.
    struct FarMemoryProvider {
        space: ReservedAddressSpace,
        exec_allocs: Vec<(usize, usize)>,
        heap_allocs: Vec<(usize, Layout)>,
    }

    impl FarMemoryProvider {
        fn allocate_heap(&mut self, size: usize, align: u64) -> io::Result<*mut u8> {
            let align = usize::try_from(align).map_err(io::Error::other)?;
            let layout =
                Layout::from_size_align(size.max(1), align.max(1)).map_err(io::Error::other)?;
            let ptr = unsafe { alloc_zeroed(layout) };
            if ptr.is_null() {
                return Err(io::Error::other("JIT allocation failed"));
            }
            self.heap_allocs.push((ptr.addr(), layout));
            Ok(ptr)
        }
    }

    impl JITMemoryProvider for FarMemoryProvider {
        fn allocate_readexec(&mut self, size: usize, align: u64) -> io::Result<*mut u8> {
            assert!(usize::try_from(align).unwrap() <= self.space.page_size);
            let len = size
                .next_multiple_of(self.space.page_size)
                .max(self.space.page_size);
            let addr = match self.exec_allocs.len() {
                0 => self.space.base,
                1 => self.space.base + self.space.len - len,
                _ => panic!("expected exactly two executable allocations"),
            };
            let result = unsafe {
                libc::mprotect(
                    addr as *mut libc::c_void,
                    len,
                    libc::PROT_READ | libc::PROT_WRITE,
                )
            };
            assert_eq!(result, 0);
            self.exec_allocs.push((addr, len));
            Ok(addr as *mut u8)
        }

        fn allocate_readwrite(&mut self, size: usize, align: u64) -> io::Result<*mut u8> {
            self.allocate_heap(size, align)
        }

        fn allocate_readonly(&mut self, size: usize, align: u64) -> io::Result<*mut u8> {
            self.allocate_heap(size, align)
        }

        unsafe fn free_memory(&mut self) {
            for (address, layout) in self.heap_allocs.drain(..) {
                unsafe { dealloc(address as *mut u8, layout) };
            }
            // Executable allocations are freed all at once when the reserved
            // address space is unmapped on drop.
            self.exec_allocs.clear();
        }

        fn finalize(&mut self, _branch_protection: BranchProtection) -> ModuleResult<()> {
            for &(addr, len) in &self.exec_allocs {
                let result = unsafe {
                    libc::mprotect(
                        addr as *mut libc::c_void,
                        len,
                        libc::PROT_READ | libc::PROT_EXEC,
                    )
                };
                assert_eq!(result, 0);
            }
            Ok(())
        }
    }

    let mut builder = JITBuilder::new(default_libcall_names()).unwrap();
    builder.memory_provider(Box::new(FarMemoryProvider {
        space: ReservedAddressSpace::new(),
        exec_allocs: Vec::new(),
        heap_allocs: Vec::new(),
    }));

    let mut module = JITModule::new(builder);
    let mut signature = module.make_signature();
    signature.returns.push(AbiParam::new(types::I32));
    let callee = module
        .declare_function("callee", Linkage::Local, &signature)
        .unwrap();
    let caller = module
        .declare_function("caller", Linkage::Local, &signature)
        .unwrap();

    // callee: mov eax, 42; ret
    module
        .define_function_bytes(callee, 1, &[0xb8, 42, 0, 0, 0, 0xc3], &[])
        .unwrap();

    // caller: call callee; ret — with the same relocation on the `call`
    // displacement that `Inst::CallKnown` emits for module-local calls.
    let relocations = [ModuleReloc {
        offset: 1,
        kind: Reloc::X86CallPCRel4,
        name: ModuleRelocTarget::user(0, callee.as_u32()),
        addend: -4,
    }];
    module
        .define_function_bytes(caller, 1, &[0xe8, 0, 0, 0, 0, 0xc3], &relocations)
        .unwrap();

    module.finalize_definitions().unwrap();

    let callee_ptr = module.get_finalized_function(callee);
    let caller_ptr = module.get_finalized_function(caller);

    // The provider must have placed the two functions out of `rel32` range of
    // each other for this test to be meaningful.
    assert!(caller_ptr.addr().abs_diff(callee_ptr.addr()) > i32::MAX as usize);

    // The `call` must have been pointed at a veneer directly behind the six
    // bytes of code, ...
    let displacement = unsafe { caller_ptr.byte_add(1).cast::<i32>().read_unaligned() };
    let veneer = caller_ptr
        .wrapping_byte_add(5)
        .wrapping_byte_offset(displacement as isize);
    assert_eq!(veneer, caller_ptr.wrapping_byte_add(6));

    // ... which consists of `jmp qword ptr [rip]` followed by the absolute
    // address of `callee`.
    let veneer_code = unsafe { std::slice::from_raw_parts(veneer, 6) };
    assert_eq!(veneer_code, [0xff, 0x25, 0, 0, 0, 0]);
    let veneer_target = unsafe { veneer.byte_add(6).cast::<u64>().read_unaligned() };
    assert_eq!(veneer_target, callee_ptr.addr() as u64);

    // Most importantly, calling `caller` must actually reach `callee`.
    let caller_fn: extern "C" fn() -> u32 = unsafe { mem::transmute(caller_ptr) };
    assert_eq!(caller_fn(), 42);

    unsafe { module.free_memory() };
}
