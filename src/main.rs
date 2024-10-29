use clang::{Entity, EntityKind, Type, TypeKind};
use std::fs::File;
use std::io::Write;
use std::collections::HashSet;

// Type mapping from C to Cangjie
fn map_c_type_to_cangjie(ty: &Type) -> String {
    match ty.get_kind() {
        TypeKind::Void => "Unit".to_string(),
        TypeKind::Bool => "Bool".to_string(),
        TypeKind::CharU | TypeKind::UChar => "UInt8".to_string(),
        TypeKind::CharS | TypeKind::SChar => "Int8".to_string(),
        TypeKind::UShort => "UInt16".to_string(),
        TypeKind::Short => "Int16".to_string(),
        TypeKind::UInt => "UInt32".to_string(),
        TypeKind::Int => "Int32".to_string(),
        TypeKind::ULong => "UInt64".to_string(),
        TypeKind::Long => "Int64".to_string(),
        TypeKind::ULongLong => "UInt64".to_string(),
        TypeKind::LongLong => "Int64".to_string(),
        TypeKind::Float => "Float32".to_string(),
        TypeKind::Double => "Float64".to_string(),
        TypeKind::Pointer => {
            let pointee = ty.get_pointee_type().unwrap();
            if pointee.get_kind() == TypeKind::CharS ||
                pointee.get_kind() == TypeKind::CharU {
                "CString".to_string()
            } else {
                format!("CPointer<{}>", map_c_type_to_cangjie(&pointee))
            }
        }
        TypeKind::Record => {
            // Get struct name
            ty.get_declaration()
                .map(|d| d.get_display_name())
                .unwrap().unwrap()
        }
        _ => "Unit /* Unsupported type */".to_string()
    }
}

// Generate Cangjie struct from C struct
fn generate_struct(entity: &Entity) -> String {
    let struct_name = entity.get_display_name().unwrap();
    let mut fields = Vec::new();

    for field in entity.get_children() {
        if field.get_kind() == EntityKind::FieldDecl {
            let field_name = field.get_display_name().unwrap();
            let field_type = field.get_type().unwrap();
            let cangjie_type = map_c_type_to_cangjie(&field_type);

            // Generate field with default value
            let default_value = match field_type.get_kind() {
                TypeKind::Bool => "false",
                TypeKind::Float | TypeKind::Double => "0.0",
                TypeKind::Pointer => "CPointer()",
                _ => "0"
            };

            fields.push(format!("    var {}: {} = {}", field_name, cangjie_type, default_value));
        }
    }

    format!("@C\nstruct {} {{\n{}\n}}", struct_name, fields.join("\n"))
}

// Generate Cangjie function declaration from C function
fn generate_function(entity: &Entity) -> String {
    let func_name = entity.get_display_name().unwrap();
    let return_type = entity.get_result_type().unwrap();
    let cangjie_return_type = map_c_type_to_cangjie(&return_type);

    let mut params = Vec::new();
    for param in entity.get_arguments().unwrap() {
        let param_name = param.get_display_name().unwrap();
        let param_type = param.get_type().unwrap();
        let cangjie_param_type = map_c_type_to_cangjie(&param_type);
        params.push(format!("{}: {}", param_name, cangjie_param_type));
    }

    format!("foreign func {}({}): {}",
            func_name,
            params.join(", "),
            cangjie_return_type)
}

pub fn generate_bindings(header_path: &str, output_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let clang = clang::Clang::new()?;
    let index = clang::Index::new(&clang, false, false);
    let tu = index.parser(header_path)
        .parse()?;

    let mut output = File::create(output_path)?;
    let mut seen_types = HashSet::new();

    // Write header comment
    writeln!(output, "// Auto-generated Cangjie bindings from {}\n", header_path)?;

    // Process all entities in the translation unit
    for entity in tu.get_entity().get_children() {
        match entity.get_kind() {
            EntityKind::StructDecl => {
                if !seen_types.contains(&entity.get_display_name()) {
                    writeln!(output, "{}\n", generate_struct(&entity))?;
                    seen_types.insert(entity.get_display_name());
                }
            }
            EntityKind::FunctionDecl => {
                writeln!(output, "{}\n", generate_function(&entity))?;
            }
            _ => {}
        }
    }

    Ok(())
}

// Example usage
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        println!("Usage: {} <input.h> <output.cj>", args[0]);
        std::process::exit(1);
    }

    generate_bindings(&args[1], &args[2])?;
    println!("Successfully generated Cangjie bindings");
    Ok(())
}