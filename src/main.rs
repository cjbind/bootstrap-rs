use clang::{Clang, Entity, EntityKind, Type};
use lazy_static::lazy_static;
use std::collections::HashSet;
use std::fmt::format;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

lazy_static! {
    static ref STRUCT_NAMES: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
    static ref ENUM_NAMES: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
}

fn translate_type(ty: Type) -> String {
    match ty.get_kind() {
        clang::TypeKind::Void => "Unit".to_string(),
        clang::TypeKind::Bool => "Bool".to_string(),
        clang::TypeKind::CharU | clang::TypeKind::UChar => "UInt8".to_string(),
        clang::TypeKind::CharS | clang::TypeKind::SChar => "Int8".to_string(),
        clang::TypeKind::UShort => "UInt16".to_string(),
        clang::TypeKind::Short => "Int16".to_string(),
        clang::TypeKind::UInt => "UInt32".to_string(),
        clang::TypeKind::Int => "Int32".to_string(),
        clang::TypeKind::ULong => "UInt64".to_string(),
        clang::TypeKind::Long => "Int64".to_string(),
        clang::TypeKind::ULongLong => "UInt64".to_string(),
        clang::TypeKind::LongLong => "Int64".to_string(),
        clang::TypeKind::Float => "Float32".to_string(),
        clang::TypeKind::Double => "Float64".to_string(),

        clang::TypeKind::Pointer => {
            let pointee = ty.get_pointee_type().unwrap();
            match pointee.get_kind() {
                clang::TypeKind::CharS => "CString".to_string(),
                clang::TypeKind::FunctionPrototype => translate_type(pointee),
                _ => format!("CPointer<{}>", translate_type(pointee)),
            }
        }
        clang::TypeKind::Elaborated => translate_type(ty.get_elaborated_type().unwrap()),
        clang::TypeKind::Record => {
            let decl = ty.get_declaration().unwrap();
            decl.get_name().unwrap().to_string()
        }
        clang::TypeKind::Typedef => {
            let underlying = ty.get_canonical_type();
            translate_type(underlying)
        }
        clang::TypeKind::Enum => {
            let decl = ty.get_declaration().unwrap();
            decl.get_name().unwrap().to_string()
        }
        clang::TypeKind::ConstantArray => {
            let element_type = ty.get_element_type().unwrap();
            let size = ty.get_size().unwrap();
            format!("VArray<{}, ${}>", translate_type(element_type), size)
        }
        clang::TypeKind::FunctionPrototype => {
            let return_type = ty.get_result_type().unwrap();
            let args: Vec<_> = ty.get_argument_types().unwrap().into_iter().collect();
            let arg_types: Vec<_> = args.iter().map(|arg| translate_type(*arg)).collect();
            format!(
                "CFunc<({}) -> {}>",
                arg_types.join(", "),
                translate_type(return_type)
            )
        }
        clang::TypeKind::FunctionNoPrototype => {
            let return_type = ty.get_result_type().unwrap();
            format!("CFunc<() -> {}>", translate_type(return_type))
        }
        _ => {
            unreachable!("Unsupported type: {:?}", ty)
        }
    }
}


fn process_enum(entity: Entity, output: &mut File) -> std::io::Result<()> {
    // 处理注释
    if let Some(comment) = entity.get_comment() {
        writeln!(output, "{}", comment)?;
    }
    let tname = entity.get_name().unwrap();
    writeln!(output, "type {} = Int32\n", tname)?;

    let mut names = ENUM_NAMES.lock().unwrap();
    names.insert(tname.clone());

    for child in entity.get_children() {
        if child.get_kind() == EntityKind::EnumConstantDecl {
            if let Some(comment) = child.get_comment() {
                writeln!(output, "{}", comment)?;
            }
            let name = child.get_name().unwrap();
            let value = child.get_enum_constant_value().unwrap().0;
            writeln!(output, "const {}: {} = {}\n", name, tname, value)?;
        }
    }
    Ok(())
}

fn process_bitfields(
    bitfields: &mut Vec<(Entity, usize)>,
    output: &mut File,
) -> std::io::Result<()> {
    let total_bits: usize = bitfields.iter().map(|(_, width)| *width as usize).sum();
    if total_bits % 8 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "位域总位数不是8的倍数",
        ));
    }
    writeln!(output, "    // 位域")?;
    for (field, width) in bitfields.drain(..) {
        let field_name = field.get_name().unwrap_or("unnamed".to_string());
        let field_type = field.get_type().unwrap().get_display_name();
        writeln!(output, "    // {} {} : {}", field_name, field_type, width)?;
    }
    writeln!(
        output,
        "    var bitfields: VArray<UInt8, ${}> = VArray<UInt8, ${}>(repeat: 0)",
        total_bits / 8,
        total_bits / 8
    )?;
    Ok(())
}

fn get_default(ty: String) -> String {
    match ty.as_str() {
        "Bool" => "false".to_string(),
        "Unit" => "()".to_string(),
        t if t.starts_with("Int") || t.starts_with("UInt") => "0".to_string(),
        t if t.starts_with("Float") => "0.0".to_string(),
        t if t.starts_with("VArray") => {
            let ty2 = t
                .strip_prefix("VArray<")
                .unwrap()
                .strip_suffix(">")
                .map(|s| s.to_string());
            format!("{}(repeat: {})", ty, get_default(ty2.unwrap()))
        }
        t if t.starts_with("CPointer") => "CPointer()".to_string(),
        t if t.starts_with("CString") => "CString(CPointer<UInt8>())".to_string(),
        t if t.starts_with("CFunc") => format!("{}(CPointer<Int8>())", t),
        t => {
            let names = ENUM_NAMES.lock().unwrap();
            if names.contains(t) {
                "0".to_string()
            } else {
                format!("{}()", t)
            }
        }
    }
}

fn process_struct(entity: Entity, output: &mut File) -> std::io::Result<()> {
    let name = entity.get_name().unwrap();

    // 处理注释
    if let Some(comment) = entity.get_comment() {
        writeln!(output, "{}", comment)?;
    }

    // 使用互斥锁添加名称
    {
        let mut names = STRUCT_NAMES.lock().unwrap();
        names.insert(name.clone());
    }

    writeln!(output, "@C")?;
    writeln!(output, "struct {} {{", name)?;

    let mut bitfields: Vec<(Entity<'_>, usize)> = Vec::new();

    for field in entity.get_children() {
        if field.get_kind() == EntityKind::FieldDecl {
            if let Some(bit_width) = field.get_bit_field_width() {
                bitfields.push((field, bit_width));
                continue;
            } else if !bitfields.is_empty() {
                process_bitfields(&mut bitfields, output)?;
            }

            let field_name = field.get_name().unwrap();
            let field_type = translate_type(field.get_type().unwrap());

            // 处理字段注释
            if let Some(comment) = field.get_comment() {
                writeln!(output, "    {}", comment)?;
            }

            writeln!(
                output,
                "    var {}: {} = {}",
                field_name,
                field_type,
                get_default(field_type.clone())
            )?;
        }
    }

    // 处理结尾的位域
    if !bitfields.is_empty() {
        process_bitfields(&mut bitfields, output)?;
    }

    writeln!(output, "}}")?;
    writeln!(output)?;

    {
        let mut names = STRUCT_NAMES.lock().unwrap();
        names.insert(name.clone());
    }
    Ok(())
}

fn process_function(entity: Entity, output: &mut File) -> std::io::Result<()> {
    let name = entity.get_name().unwrap();
    let return_type = translate_type(entity.get_result_type().unwrap());

    // 处理注释
    if let Some(comment) = entity.get_comment() {
        writeln!(output, "{}", comment)?;
    }

    write!(output, "foreign func {}(", name)?;

    let args: Vec<_> = entity.get_arguments().unwrap().into_iter().collect();
    for (i, arg) in args.iter().enumerate() {
        let mut arg_name = arg.get_name().unwrap_or(format!("arg{}", i));
        if arg_name == "Unit" || arg_name == "type" {
            arg_name = format!("{}_", arg_name);
        }
        let arg_type = translate_type(arg.get_type().unwrap());

        write!(output, "{}: {}", arg_name, arg_type)?;
        if i < args.len() - 1 {
            write!(output, ", ")?;
        }
    }

    writeln!(output, "): {}", return_type)?;
    writeln!(output)?;
    Ok(())
}

struct typdef {
    name: String,
    typ: String,
    comment: Option<String>,
}

static mut TYPEDEFS: Vec<typdef> = Vec::new();

fn process_typedef(entity: Entity) -> std::io::Result<()> {
    let name = entity.get_name().unwrap();
    let underlying_type = entity.get_typedef_underlying_type().unwrap();

    if name == translate_type(underlying_type) {
        return Ok(());
    }

    let t = typdef {
        name: name,
        typ: translate_type(underlying_type),
        comment: entity.get_comment(),
    };
    unsafe {
        TYPEDEFS.push(t);
    }

    Ok(())
}

fn generate_typedefs(output: &mut File) -> std::io::Result<()> {
    let names = STRUCT_NAMES.lock().unwrap();
    for t in unsafe { TYPEDEFS.iter() } {
        if !names.contains(&t.typ) {
            continue;
        }

        if let Some(comment) = &t.comment {
            writeln!(output, "{}", comment)?;
        }
        writeln!(output, "type {} = {}", t.name, t.typ)?;
    }

    Ok(())
}

pub fn generate_bindings(
    header_path: &str,
    output_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let clang = Clang::new()?;
    let index = clang::Index::new(&clang, false, true);
    let tu = index
        .parser(header_path)
        .arguments(&["-I", "./include"])
        .detailed_preprocessing_record(false)
        .skip_function_bodies(true)
        .parse()?;

    let mut output = File::create(Path::new(output_path))?;

    // 写入文件头
    writeln!(
        output,
        "// This file is automatically generated. DO NOT EDIT."
    )?;
    writeln!(output)?;
    writeln!(output, "package clang_cj")?;

    for entity in tu.get_entity().get_children() {
        if entity.is_in_system_header() {
            continue;
        }

        match entity.get_kind() {
            EntityKind::EnumDecl => process_enum(entity, &mut output)?,
            EntityKind::FunctionDecl => process_function(entity, &mut output)?,
            EntityKind::TypedefDecl => process_typedef(entity)?,
            _ => (),
        }
    }

    for entity in tu.get_entity().get_children() {
        if entity.is_in_system_header() {
            continue;
        }

        match entity.get_kind() {
            EntityKind::StructDecl => process_struct(entity, &mut output)?,
            _ => (),
        }
    }

    generate_typedefs(&mut output)?;

    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <input.h> <output.cj>", args[0]);
        std::process::exit(1);
    }

    match generate_bindings(&args[1], &args[2]) {
        Ok(_) => println!("Successfully generated bindings"),
        Err(e) => {
            eprintln!("Error generating bindings: {}", e);
            std::process::exit(1);
        }
    }
}
