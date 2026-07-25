use cranelift_codegen::ir::*;
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings;
use cranelift_codegen::{Context, ir::types::I16};
use cranelift_entity::EntityRef;
use cranelift_frontend::*;
use cranelift_module::*;
use cranelift_object::*;

#[test]
fn error_on_incompatible_sig_in_declare_function() {
    let flag_builder = settings::builder();
    let isa_builder = cranelift_codegen::isa::lookup_by_name("x86_64-unknown-linux-gnu").unwrap();
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .unwrap();
    let mut module =
        ObjectModule::new(ObjectBuilder::new(isa, "foo", default_libcall_names()).unwrap());
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

fn define_simple_function(module: &mut ObjectModule) -> FuncId {
    let sig = Signature {
        params: vec![],
        returns: vec![],
        call_conv: CallConv::SystemV,
    };

    let func_id = module
        .declare_function("abc", Linkage::Local, &sig)
        .unwrap();

    let mut ctx = Context::new();
    ctx.func = Function::with_name_signature(UserFuncName::user(0, func_id.as_u32()), sig);
    let mut func_ctx = FunctionBuilderContext::new();
    {
        let mut bcx: FunctionBuilder = FunctionBuilder::new(&mut ctx.func, &mut func_ctx);
        let block = bcx.create_block();
        bcx.switch_to_block(block);
        bcx.ins().return_(&[]);
    }

    module.define_function(func_id, &mut ctx).unwrap();

    func_id
}

#[test]
#[should_panic(expected = "Result::unwrap()` on an `Err` value: DuplicateDefinition(\"abc\")")]
fn panic_on_define_after_finalize() {
    let flag_builder = settings::builder();
    let isa_builder = cranelift_codegen::isa::lookup_by_name("x86_64-unknown-linux-gnu").unwrap();
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .unwrap();
    let mut module =
        ObjectModule::new(ObjectBuilder::new(isa, "foo", default_libcall_names()).unwrap());

    define_simple_function(&mut module);
    define_simple_function(&mut module);
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "checks a debug assertion")]
#[should_panic(expected = "function \"abc\" with linkage Local must be defined but is not")]
fn panic_on_declare_without_define() {
    let flag_builder = settings::builder();
    let isa_builder = cranelift_codegen::isa::lookup_by_name("x86_64-unknown-linux-gnu").unwrap();
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .unwrap();
    let mut module =
        ObjectModule::new(ObjectBuilder::new(isa, "foo", default_libcall_names()).unwrap());

    module
        .declare_function("abc", Linkage::Local, &Signature::new(CallConv::SystemV))
        .unwrap();

    module.finish();
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
    let flag_builder = settings::builder();
    let isa_builder = cranelift_codegen::isa::lookup_by_name("x86_64-unknown-linux-gnu").unwrap();
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .unwrap();
    let mut module =
        ObjectModule::new(ObjectBuilder::new(isa, "foo", default_libcall_names()).unwrap());

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

    module.define_function(func_id, &mut ctx).unwrap();

    module.finish();
}

#[test]
#[should_panic(expected = "has a null byte, which is disallowed")]
fn reject_nul_byte_symbol_for_func() {
    let flag_builder = settings::builder();
    let isa_builder = cranelift_codegen::isa::lookup_by_name("x86_64-unknown-linux-gnu").unwrap();
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .unwrap();
    let mut module =
        ObjectModule::new(ObjectBuilder::new(isa, "foo", default_libcall_names()).unwrap());

    let sig = Signature {
        params: vec![],
        returns: vec![],
        call_conv: CallConv::SystemV,
    };

    let _ = module
        .declare_function("function\u{0}with\u{0}nul\u{0}bytes", Linkage::Local, &sig)
        .unwrap();
}

#[test]
#[should_panic(expected = "has a null byte, which is disallowed")]
fn reject_nul_byte_symbol_for_data() {
    let flag_builder = settings::builder();
    let isa_builder = cranelift_codegen::isa::lookup_by_name("x86_64-unknown-linux-gnu").unwrap();
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .unwrap();
    let mut module =
        ObjectModule::new(ObjectBuilder::new(isa, "foo", default_libcall_names()).unwrap());

    let _ = module
        .declare_data(
            "data\u{0}with\u{0}nul\u{0}bytes",
            Linkage::Local,
            /* writable = */ true,
            /* tls = */ false,
        )
        .unwrap();
}

#[test]
fn x86_64_colocated_symbol_address_uses_non_branch_relocation() {
    use object::{Object as _, ObjectSection as _, ObjectSymbol as _};

    for (triple, expected_encoding) in [
        (
            "x86_64-unknown-linux-gnu",
            object::RelocationEncoding::Generic,
        ),
        (
            "x86_64-apple-darwin",
            object::RelocationEncoding::X86RipRelative,
        ),
        (
            "x86_64-pc-windows-msvc",
            object::RelocationEncoding::Generic,
        ),
    ] {
        let flag_builder = settings::builder();
        let isa = cranelift_codegen::isa::lookup_by_name(triple)
            .unwrap()
            .finish(settings::Flags::new(flag_builder))
            .unwrap();
        let mut module =
            ObjectModule::new(ObjectBuilder::new(isa, "test", default_libcall_names()).unwrap());

        let data_id = module
            .declare_data("my_data", Linkage::Local, true, false)
            .unwrap();
        let mut data_desc = DataDescription::new();
        data_desc.define_zeroinit(8);
        module.define_data(data_id, &data_desc).unwrap();

        let sig = Signature {
            params: vec![],
            returns: vec![AbiParam::new(types::I64)],
            call_conv: module.target_config().default_call_conv,
        };
        let func_id = module
            .declare_function("data_address", Linkage::Local, &sig)
            .unwrap();
        let mut ctx = Context::new();
        ctx.func = Function::with_name_signature(UserFuncName::user(0, func_id.as_u32()), sig);
        let mut func_ctx = FunctionBuilderContext::new();
        {
            let mut bcx = FunctionBuilder::new(&mut ctx.func, &mut func_ctx);
            let block = bcx.create_block();
            bcx.switch_to_block(block);
            let data = module.declare_data_in_func(data_id, &mut bcx.func);
            let address = bcx
                .ins()
                .symbol_value(module.target_config().pointer_type(), data);
            bcx.ins().return_(&[address]);
            bcx.seal_all_blocks();
            bcx.finalize();
        }
        module.define_function(func_id, &mut ctx).unwrap();

        let bytes = module.finish().emit().expect("emit object file");
        let file = object::File::parse(&*bytes).expect("parse emitted object");
        let data_symbol = file
            .symbols()
            .find(|symbol| {
                symbol
                    .name()
                    .is_ok_and(|name| name.trim_start_matches('_') == "my_data")
            })
            .unwrap_or_else(|| panic!("{triple}: data symbol present in object"))
            .index();
        let function = file
            .symbols()
            .find(|symbol| {
                symbol
                    .name()
                    .is_ok_and(|name| name.trim_start_matches('_') == "data_address")
            })
            .unwrap_or_else(|| panic!("{triple}: function symbol present in object"));
        let section = file
            .section_by_index(function.section_index().unwrap())
            .expect("function section present");
        let (_, relocation) = section
            .relocations()
            .find(|(_, relocation)| {
                relocation.target() == object::read::RelocationTarget::Symbol(data_symbol)
            })
            .expect("address materialization relocation present");

        assert_eq!(
            relocation.kind(),
            object::RelocationKind::Relative,
            "{triple}"
        );
        assert_eq!(
            relocation.encoding(),
            expected_encoding,
            "{triple}: an address materialization must not use a branch relocation"
        );
        assert_eq!(relocation.size(), 32, "{triple}");
    }
}
