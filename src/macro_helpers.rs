// black magic purification...
pub fn purify_struct_name(s: &str) -> String {
    let s = String::from(s);
    let name = s.split("::")
        .collect::<Vec<&str>>()
        .into_iter().rev()
        .take(2).rev()
        .next()
        .unwrap_or("");

    String::from(name)
}

#[macro_export]
macro_rules! function {
    () => {{
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str {
            std::any::type_name::<T>()
        }
        let name = type_name_of(f);
        &name[..name.len() - 3]
    }}
}

// use black magic to pull current structure name
#[macro_export]
macro_rules! struct_name {
    () => {
        fn _name() -> String {
            $crate::macro_helpers::purify_struct_name(function!())
        }
    }
}

// simply forward previously pulled structure name inside _name() to a current trait's impl name().
#[macro_export]
macro_rules! name {
    () => {
        fn name(&self) -> String {
            Self::_name()
        }
    }
}