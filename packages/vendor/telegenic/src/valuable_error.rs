/// Implements `valuable::Valuable` (and `Structable`) for a `thiserror` enum as
/// a flat struct: a `message` field holding the `Display` string, followed by
/// the active variant's fields for the variants listed. Unlisted variants
/// (tuple/unit, or ones whose payload isn't itself `Valuable`) render as just
/// the message.
macro_rules! error_valuable {
    ($ty:ty, $name:literal $(, $vn:ident { $($f:ident),* $(,)? } )* $(,)?) => {
        impl valuable::Valuable for $ty {
            fn as_value(&self) -> valuable::Value<'_> {
                valuable::Value::Structable(self)
            }
            fn visit(&self, visit: &mut dyn valuable::Visit) {
                let message = self.to_string();
                match self {
                    $( Self::$vn { $($f),* } => {
                        const F: &[valuable::NamedField<'static>] =
                            &[valuable::NamedField::new("message")
                              $(, valuable::NamedField::new(stringify!($f)))*];
                        let values = [valuable::Value::String(&message)
                            $(, valuable::Valuable::as_value($f))*];
                        visit.visit_named_fields(&valuable::NamedValues::new(F, &values));
                    } )*
                    #[allow(unreachable_patterns)]
                    _ => {
                        const F: &[valuable::NamedField<'static>] =
                            &[valuable::NamedField::new("message")];
                        visit.visit_named_fields(
                            &valuable::NamedValues::new(F, &[valuable::Value::String(&message)]),
                        );
                    }
                }
            }
        }
        impl valuable::Structable for $ty {
            fn definition(&self) -> valuable::StructDef<'_> {
                match self {
                    $( Self::$vn { .. } => {
                        const F: &[valuable::NamedField<'static>] =
                            &[valuable::NamedField::new("message")
                              $(, valuable::NamedField::new(stringify!($f)))*];
                        valuable::StructDef::new_dynamic($name, valuable::Fields::Named(F))
                    } )*
                    #[allow(unreachable_patterns)]
                    _ => {
                        const F: &[valuable::NamedField<'static>] =
                            &[valuable::NamedField::new("message")];
                        valuable::StructDef::new_dynamic($name, valuable::Fields::Named(F))
                    }
                }
            }
        }
    };
}
