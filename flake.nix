{
  description = "smkb — software KVM (mouse + keyboard) over WiFi, Hyprland edition";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems f;

      packagesFor =
        system:
        let
          pkgs = import nixpkgs { inherit system; };

          common = {
            version = "0.1.0";
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ pkgs.pkg-config ];
            meta.license = pkgs.lib.licenses.mit;
          };

          smkb-master = pkgs.rustPlatform.buildRustPackage (
            common
            // {
              pname = "smkb-master";
              cargoBuildFlags = [
                "-p"
                "smkb-master"
              ];
              meta.description = "smkb master: runs on the PC whose mouse/keyboard are shared";
            }
          );

          smkb-slave = pkgs.rustPlatform.buildRustPackage (
            common
            // {
              pname = "smkb-slave";
              cargoBuildFlags = [
                "-p"
                "smkb-slave"
              ];
              meta.description = "smkb slave: runs on the PC that receives the shared mouse/keyboard";
            }
          );
        in
        {
          inherit smkb-master smkb-slave;
          default = smkb-master;
        };
    in
    {
      packages = forAllSystems packagesFor;

      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              rustc
              rust-analyzer
              clippy
              rustfmt
              socat
              pkg-config
            ];
            RUST_LOG = "smkb_master=debug,smkb_slave=debug,smkb_platform=debug,info";
          };
        }
      );

      nixosModules.smkb =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        with lib;
        let
          cfg = config.services.smkb;
          forSystem = pkgs.system;
        in
        {
          options.services.smkb = {
            master = {
              enable = mkEnableOption "smkb-master (the PC whose input is shared)";
              user = mkOption {
                type = types.str;
                description = "User to run smkb-master's systemd --user service as.";
              };
            };
            slave = {
              enable = mkEnableOption "smkb-slave (the PC that receives the shared input)";
              user = mkOption {
                type = types.str;
                description = "User to run smkb-slave's systemd --user service as.";
              };
            };
          };

          config = mkIf (cfg.master.enable || cfg.slave.enable) {
            services.udev.extraRules = ''
              KERNEL=="uinput", GROUP="input", MODE="0660"
            '';
            users.users = mkMerge [
              (mkIf cfg.master.enable { ${cfg.master.user}.extraGroups = [ "input" ]; })
              (mkIf cfg.slave.enable { ${cfg.slave.user}.extraGroups = [ "input" ]; })
            ];

            systemd.user.services = mkMerge [
              (mkIf cfg.master.enable {
                smkb-master = {
                  description = "smkb-master";
                  wantedBy = [ "graphical-session.target" ];
                  after = [ "graphical-session.target" ];
                  serviceConfig = {
                    ExecStart = "${self.packages.${forSystem}.smkb-master}/bin/smkb-master";
                    Restart = "on-failure";
                    RestartSec = 2;
                  };
                };
              })
              (mkIf cfg.slave.enable {
                smkb-slave = {
                  description = "smkb-slave";
                  wantedBy = [ "graphical-session.target" ];
                  after = [ "graphical-session.target" ];
                  serviceConfig = {
                    ExecStart = "${self.packages.${forSystem}.smkb-slave}/bin/smkb-slave";
                    Restart = "on-failure";
                    RestartSec = 2;
                  };
                };
              })
            ];
          };
        };
    };
}
