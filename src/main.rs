use clang::{Entity, EntityKind, Type, TypeKind};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;

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
        TypeKind::Typedef => {
            // let canonical = ty.get_canonical_type();
            // map_c_type_to_cangjie(&canonical)
            ty.get_display_name()
        }
        TypeKind::Enum => "Int32".to_string(),
        TypeKind::Record => ty.get_display_name(),
        TypeKind::Elaborated => {
            ty.get_display_name()
        }
        _ => format!("/* Unsupported type: {:?} {:?} */", ty.get_kind(), ty.get_display_name()),
    }
}

struct BindingGenerator {
    processed_types: HashSet<String>,
    type_aliases: HashMap<String, String>,
    output: String,
}

impl BindingGenerator {
    fn new() -> Self {
        BindingGenerator {
            processed_types: HashSet::new(),
            type_aliases: HashMap::new(),
            output: String::new(),
        }
    }

    fn process_enum(&mut self, entity: Entity) {
        if self.processed_types.contains(&entity.get_name().unwrap_or_default()) {
            return;
        }

        let name = entity.get_name().unwrap();
        self.processed_types.insert(name.clone());

        let mut enum_def = format!("enum {} {{\n", name);

        for child in entity.get_children() {
            if let Some(val) = child.get_enum_constant_value() {
                enum_def.push_str(&format!("    {} = {}\n",
                                           child.get_name().unwrap(),
                                           val.0));
            }
        }
        enum_def.push_str("}\n\n");
        self.output.push_str(&enum_def);
    }

    fn process_struct(&mut self, entity: Entity) {
        if self.processed_types.contains(&entity.get_name().unwrap_or_default()) {
            return;
        }

        let name = entity.get_name().unwrap();
        self.processed_types.insert(name.clone());

        let mut struct_def = format!("@C\nstruct {} {{\n", name);

        for field in entity.get_children() {
            if field.get_kind() == EntityKind::FieldDecl {
                let field_name = field.get_name().unwrap_or("unknown1".to_string());
                let field_type = field.get_type().unwrap();
                let cangjie_type = map_c_type_to_cangjie(&field_type);

                // Handle arrays
                if field_type.get_kind() == TypeKind::ConstantArray {
                    let size = field_type.get_size().unwrap_or(0);
                    let element_type = field_type.get_element_type().unwrap();
                    struct_def.push_str(&format!("    var {} = VArray<{}, ${}>(repeat: 0)\n",
                                                 field_name,
                                                 map_c_type_to_cangjie(&element_type),
                                                 size));
                } else {
                    struct_def.push_str(&format!("    var {}: {} \n",
                                                 field_name,
                                                 cangjie_type,
                    ));
                }
            }
        }
        struct_def.push_str("}\n\n");
        self.output.push_str(&struct_def);
    }

    fn process_function(&mut self, entity: Entity) {
        let func_name = entity.get_name().unwrap();
        let return_type = entity.get_result_type().unwrap();
        let cangjie_return_type = map_c_type_to_cangjie(&return_type);

        let mut func_def = format!("foreign func {}(", func_name);

        // Process parameters
        let mut first = true;
        for param in entity.get_arguments().unwrap() {
            if !first {
                func_def.push_str(", ");
            }
            first = false;

            let param_name = param.get_name().unwrap_or("arg".to_string());
            let param_type = param.get_type().unwrap();
            let cangjie_param_type = map_c_type_to_cangjie(&param_type);

            func_def.push_str(&format!("{}: {}", param_name, cangjie_param_type));
        }

        func_def.push_str(&format!("): {}\n", cangjie_return_type));
        self.output.push_str(&func_def);
    }

    fn process_typedef(&mut self, entity: Entity) {
        let name = entity.get_name().unwrap();
        let underlying_type = entity.get_type().unwrap().get_canonical_type();
        let cangjie_type = map_c_type_to_cangjie(&underlying_type);

        self.type_aliases.insert(name.clone(), cangjie_type.clone());
    }

    fn generate_bindings(&mut self, header_path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let clang = clang::Clang::new()?;
        let index = clang::Index::new(&clang, true, true);
        let tu = index.parser(header_path)
            .arguments(&["-I", "T:\\cjbind-bootstrap\\include"])
            .detailed_preprocessing_record(true)
            .skip_function_bodies(true)
            .parse()?;

        let entity = tu.get_entity();

        // Header
        self.output.push_str("// Generated by C-to-Cangjie binding generator\n\n");

        // generate actual bindings
        for child in entity.get_children() {
            match child.get_kind() {
                EntityKind::TypedefDecl => self.process_typedef(child),
                EntityKind::EnumDecl => self.process_enum(child),
                EntityKind::StructDecl => self.process_struct(child),
                EntityKind::FunctionDecl => self.process_function(child),
                _ => {}
            }
        }

        Ok(())
    }

    fn write_to_file(&self, output_path: &str) -> std::io::Result<()> {
        let mut file = File::create(output_path)?;
        file.write_all(self.output.as_bytes())?;
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <input.h> <output.cj>", args[0]);
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = &args[2];

    let mut generator = BindingGenerator::new();
    generator.generate_bindings(input_path)?;
    generator.write_to_file(output_path)?;

    println!("Successfully generated bindings: {}", output_path);
    Ok(())
}