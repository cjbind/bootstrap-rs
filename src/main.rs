use clang::{Clang, Entity, EntityKind, Type};
use std::fs::File;
use std::io::Write;
use std::path::Path;

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
            if pointee.get_kind() == clang::TypeKind::CharS {
                "CString".to_string()
            } else {
                format!("CPointer<{}>", translate_type(pointee))
            }
        }
        clang::TypeKind::Elaborated => translate_type(ty.get_elaborated_type().unwrap()),
        clang::TypeKind::Record => {
            let decl = ty.get_declaration().unwrap();
            decl.get_name().unwrap().to_string()
        }
        _ => {
            unreachable!("Unsupported type: {:?}", ty)
        }
    }
}

fn process_enum(entity: Entity, output: &mut File) -> std::io::Result<()> {
    // 处理注释
    if let Some(comment) = entity.get_comment() {
        writeln!(output, "// {}", comment)?;
    }

    for child in entity.get_children() {
        if child.get_kind() == EntityKind::EnumConstantDecl {
            let name = child.get_name().unwrap();
            let value = child.get_enum_constant_value().unwrap().0;
            writeln!(output, "let {} = {}", name, value)?;
        }
    }
    Ok(())
}

fn process_struct(entity: Entity, output: &mut File) -> std::io::Result<()> {
    let name = entity.get_name().unwrap_or("Anonymous".to_string());

    // 处理注释
    if let Some(comment) = entity.get_comment() {
        writeln!(output, "// {}", comment)?;
    }

    writeln!(output, "@C")?;
    writeln!(output, "struct {} {{", name)?;

    for field in entity.get_children() {
        if field.get_kind() == EntityKind::FieldDecl {
            let field_name = field.get_name().unwrap_or_else(||
                {
                    println!("Field without name in struct {}", name);
                    "Anonymous".to_string()
                }
            );
            let field_type = translate_type(field.get_type().unwrap());

            // 处理字段注释
            if let Some(comment) = field.get_comment() {
                writeln!(output, "    {}", comment)?;
            }

            match field_type.as_str() {
                "Bool" => writeln!(output, "    var {}: {} = false", field_name, field_type)?,
                "Unit" => writeln!(output, "    var {}: {}", field_name, field_type)?,
                t if t.starts_with("Float") => writeln!(output, "    var {}: {} = 0.0", field_name, field_type)?,
                _ => writeln!(output, "    var {}: {} = 0", field_name, field_type)?
            }
        }
    }

    writeln!(output, "}}")?;
    writeln!(output)?;
    Ok(())
}

fn process_function(entity: Entity, output: &mut File) -> std::io::Result<()> {
    let name = entity.get_name().unwrap();
    let return_type = translate_type(entity.get_result_type().unwrap());

    // 处理注释
    if let Some(comment) = entity.get_comment() {
        writeln!(output, "// {}", comment)?;
    }

    write!(output, "foreign func {}(", name)?;

    let args: Vec<_> = entity.get_arguments().unwrap().into_iter().collect();
    for (i, arg) in args.iter().enumerate() {
        let arg_name = arg.get_name().unwrap_or(format!("arg{}", i));
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

fn process_typedef(entity: Entity, output: &mut File) -> std::io::Result<()> {
    let name = entity.get_name().unwrap();
    let underlying_type = entity.get_typedef_underlying_type().unwrap();

    // 处理注释
    if let Some(comment) = entity.get_comment() {
        writeln!(output, "// {}", comment)?;
    }

    writeln!(output, "type {} = {}", name, translate_type(underlying_type))?;
    writeln!(output)?;
    Ok(())
}

pub fn generate_bindings(header_path: &str, output_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let clang = Clang::new()?;
    let index = clang::Index::new(&clang, false, true);
    let tu = index.parser(header_path)
        .arguments(&["-I", "T:\\cjbind-bootstrap\\include"])
        .detailed_preprocessing_record(true)
        .skip_function_bodies(true)
        .parse()?;

    let mut output = File::create(Path::new(output_path))?;

    // 写入文件头
    writeln!(output, "// This file is automatically generated. DO NOT EDIT.")?;
    writeln!(output)?;

    for entity in tu.get_entity().get_children() {
        match entity.get_kind() {
            EntityKind::EnumDecl => process_enum(entity, &mut output)?,
            EntityKind::StructDecl => process_struct(entity, &mut output)?,
            EntityKind::FunctionDecl => process_function(entity, &mut output)?,
            EntityKind::TypedefDecl => process_typedef(entity, &mut output)?,
            _ => ()
        }
    }

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