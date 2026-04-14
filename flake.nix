{
  description = "Gizmo Engine - A custom 3D game engine and physics simulation framework in Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      devShells = forAllSystems (system:
        let
          overlays = [ (import rust-overlay) ];
          pkgs = import nixpkgs { inherit system overlays; };

          rustToolchain = pkgs.rust-bin.stable.latest.default.override {
            extensions = [ "rust-src" "rust-analyzer" ];
          };

          # Linux-specific dependencies
          linuxBuildInputs = with pkgs; [
            # Vulkan
            vulkan-loader
            vulkan-headers
            vulkan-validation-layers
            vulkan-tools
            shaderc

            # X11
            xorg.libX11
            xorg.libXcursor
            xorg.libXrandr
            xorg.libXi
            xorg.libxcb

            # Wayland
            wayland
            libxkbcommon

            # Audio (rodio/cpal)
            alsa-lib

            # Misc
            libGL
          ];

          linuxShellHook = ''
            export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath linuxBuildInputs}:$LD_LIBRARY_PATH"
            export VK_LAYER_PATH="${pkgs.vulkan-validation-layers}/share/vulkan/explicit_layer.d"
          '';

          isLinux = pkgs.stdenv.hostPlatform.isLinux;
          isDarwin = pkgs.stdenv.hostPlatform.isDarwin;
        in
        {
          default = pkgs.mkShell {
            buildInputs = [
              rustToolchain
              pkgs.pkg-config
            ]
            ++ pkgs.lib.optionals isLinux linuxBuildInputs
            ++ pkgs.lib.optionals isDarwin [ pkgs.apple-sdk_15 ];

            shellHook = pkgs.lib.optionalString isLinux linuxShellHook;
          };
        }
      );
    };
}
