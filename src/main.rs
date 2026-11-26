//! The executable. See [`feljosit::cli`].

fn main() {
    std::process::exit(feljosit::cli::main(std::env::args().skip(1).collect()));
}
