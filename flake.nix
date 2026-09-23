{
  description = "dank-pinentry - a pinentry with a TTY frontend and a DankMaterialShell plugin frontend";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = nixpkgs.legacyPackages.${system};

      dank-pinentry = pkgs.rustPlatform.buildRustPackage {
        pname = "dank-pinentry";
        version = "0.1.0";
        src = ./.;

        # Using the checked-in lock file avoids having to track a vendor hash.
        cargoLock.lockFile = ./Cargo.lock;

        # The pty-based tests need a terminal the sandbox does not provide;
        # the unit and transcript tests run fine.
        checkFlags = [];

        meta = with pkgs.lib; {
          description = "A pinentry with a TTY frontend and a DankMaterialShell plugin frontend";
          homepage = "https://github.com/augustocdias/dank-pinentry";
          license = licenses.mit;
          mainProgram = "dank-pinentry";
          platforms = platforms.linux;
        };
      };

      # The QML half. Installed as a DMS plugin directory.
      dank-pinentry-plugin = pkgs.stdenvNoCC.mkDerivation {
        pname = "dank-pinentry-plugin";
        version = "0.1.0";
        src = ./plugin;

        dontBuild = true;

        installPhase = ''
          runHook preInstall
          mkdir -p "$out/share/dms-plugins/dankbarPinentry"
          cp -r ./* "$out/share/dms-plugins/dankbarPinentry/"
          runHook postInstall
        '';

        meta = with pkgs.lib; {
          description = "DankMaterialShell plugin rendering dank-pinentry prompts";
          homepage = "https://github.com/augustocdias/dank-pinentry";
          license = licenses.mit;
          platforms = platforms.linux;
        };
      };
    in {
      packages = {
        inherit dank-pinentry dank-pinentry-plugin;
        default = dank-pinentry;
      };

      apps.default = flake-utils.lib.mkApp {drv = dank-pinentry;};

      devShells.default = pkgs.mkShell {
        packages = with pkgs; [
          cargo
          rustc
          rustfmt
          clippy
          rust-analyzer

          # Needed by the integration test scripts.
          gnupg
          python3
          netcat-openbsd

          # `gpg-error <code>` for checking wire values by hand.
          libgpg-error

          # QML tooling for the plugin half.
          qt6.qtdeclarative
        ];

        shellHook = ''
          echo "dank-pinentry dev shell"
          echo "  cargo test                       unit + transcript tests"
          echo "  python3 scripts/test-tty.py      TTY frontend, via a pty"
          echo "  python3 scripts/test-gpg-auto.py full end-to-end with a real gpg-agent"
          echo "  ./scripts/test-gpg-integration.sh dms   interactive, uses the DMS plugin"
        '';
      };

      formatter = pkgs.alejandra;
    })
    // {
      homeModules.default = {
        config,
        lib,
        pkgs,
        options,
        ...
      }: let
        cfg = config.programs.dank-pinentry;
        inherit (lib) mkEnableOption mkOption mkIf types;

        pluginSrc = "${self.packages.${pkgs.system}.dank-pinentry-plugin}/share/dms-plugins/dankbarPinentry";
        # DMS's home-manager module goes by three names depending on where it
        # comes from (stable flake, flake, nixpkgs); the plugin registry's
        # module probes the same way.
        dmsPluginOptions = lib.filter (path: lib.hasAttrByPath path options) [
          ["programs" "dank-material-shell" "plugins"]
          ["programs" "dankMaterialShell" "plugins"]
          ["programs" "dms-shell" "plugins"]
        ];
      in {
        options.programs.dank-pinentry = {
          enable = mkEnableOption "dank-pinentry";

          package = mkOption {
            type = types.package;
            default = self.packages.${pkgs.system}.dank-pinentry;
            description = "The dank-pinentry package to use.";
          };

          installPlugin = mkOption {
            type = types.bool;
            default = true;
            description = ''
              Install and enable the DankMaterialShell plugin by setting
              `plugins.dankbarPinentry.src` in DMS's own home-manager module, so
              DMS also writes the `enabled` flag it needs. Plugin settings
              still go in `plugins.dankbarPinentry.settings` there.

              Without DMS's module the plugin is only linked into
              ~/.config/DankMaterialShell/plugins, and must be enabled once
              with `dms ipc call plugins enable dankbarPinentry`.
            '';
          };

          configureGpgAgent = mkOption {
            type = types.bool;
            default = false;
            description = ''
              Append `pinentry-program` to gpg-agent.conf via
              `services.gpg-agent.extraConfig`.

              Off by default because many configurations already set
              `pinentry-program` (or `services.gpg-agent.pinentry.package`),
              and two sources writing the same key is worse than writing it
              yourself. The line needed is:

                pinentry-program ''${package}/bin/dank-pinentry

              Requires `services.gpg-agent.enable`.
            '';
          };

          ui = mkOption {
            type = types.enum ["auto" "tty" "dms"];
            default = "auto";
            description = ''
              Which frontend to use. `auto` prefers a terminal whenever one is
              usable and falls back to the shell plugin.
            '';
          };

          socketPath = mkOption {
            type = types.nullOr types.str;
            default = null;
            example = "/run/user/1000/dms-pinentry.sock";
            description = "Override the socket the plugin listens on.";
          };

          maskChar = mkOption {
            type = types.nullOr types.str;
            default = null;
            example = "•";
            description = "Character drawn per typed character in the terminal frontend.";
          };

          timeout = mkOption {
            type = types.nullOr types.ints.unsigned;
            default = null;
            example = 120;
            description = ''
              Seconds before an unanswered prompt cancels itself; 0 never does.
              Defaults to 60. gpg-agent's `pinentry-timeout`, if set, wins.
            '';
          };
        };

        config = mkIf cfg.enable (lib.mkMerge (
          [
            {
              home.packages = [cfg.package];

              xdg.configFile."dank-pinentry/config.toml".text = lib.concatStringsSep "\n" (
                ["ui = \"${cfg.ui}\""]
                ++ lib.optional (cfg.socketPath != null) "socket_path = \"${cfg.socketPath}\""
                ++ lib.optional (cfg.maskChar != null) "mask_char = \"${cfg.maskChar}\""
                ++ lib.optional (cfg.timeout != null) "timeout = ${toString cfg.timeout}"
                ++ [""]
              );

              # Appending through the gpg-agent module rather than writing
              # ~/.gnupg/gpg-agent.conf directly: that file is very often
              # already managed, and taking it over would silently drop the
              # user's other agent settings.
              services.gpg-agent.extraConfig = mkIf cfg.configureGpgAgent ''
                pinentry-program ${cfg.package}/bin/dank-pinentry
              '';
            }

            (mkIf (cfg.installPlugin && dmsPluginOptions == []) {
              xdg.configFile."DankMaterialShell/plugins/dankbarPinentry".source = pluginSrc;
            })
          ]
          ++ map (path:
            mkIf cfg.installPlugin (lib.setAttrByPath path {dankbarPinentry.src = pluginSrc;}))
          dmsPluginOptions
        ));
      };
    };
}
