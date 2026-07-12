{ pkgs ? import <nixpkgs> {} }:

let
  # Structural build-time and runtime dependencies
  buildDeps = with pkgs; [
    atk
    cairo
    expat
    fontconfig
    freetype
    gdk-pixbuf
    glib
    glibc
    gst_all_1.gstreamer
    gst_all_1.gst-plugins-base
    gtk3
    harfbuzz
    libGL
    libxkbcommon
    noto-fonts-color-emoji
    pango
    wayland
    xdotool
    xorg.libX11
    xorg.libXcursor
    xorg.libXrandr
    xorg.libXi
    # Webcam dependencies
    libclang
    linuxHeaders
    v4l-utils
    # Microphone dependencies
    alsa-lib
  ];

  # Helper function to safely read the .dev attribute, falling back to the base package
  getDev = pkg: pkg.dev or pkg;
in
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    pkg-config
  ];

  # Pull in the Rust toolchain + your dependencies
  buildInputs = with pkgs; [
    rustc
    cargo
    rustfmt
    clippy
  ] ++ buildDeps;

  shellHook = ''
    export PKG_CONFIG_PATH="${pkgs.lib.makeSearchPath "lib/pkgconfig" (map getDev buildDeps)}"
    export LD_LIBRARY_PATH="/run/opengl-driver/lib:${pkgs.lib.makeLibraryPath buildDeps}:$LD_LIBRARY_PATH"
    export LIBCLANG_PATH="${pkgs.libclang.lib}/lib"
    export BINDGEN_EXTRA_CLANG_ARGS="-I${pkgs.glibc.dev}/include -I${pkgs.linuxHeaders}/include"

    # Create a fontconfig that includes both system fonts and emoji fonts
    export FONTCONFIG_FILE="${pkgs.makeFontsConf {
      fontDirectories = [
        pkgs.noto-fonts-color-emoji
        pkgs.dejavu_fonts
        pkgs.liberation_ttf
      ];
    }}"

    echo "LBH development environment loaded"
    echo "Run 'cargo build' to build the project"
    echo "Run 'cargo run' to run the application"
  '';
}
