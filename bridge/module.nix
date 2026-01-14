flake:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib)
    mkEnableOption
    mkOption
    types
    mkIf
    literalExpression
    ;

  cfg = config.services.fcm2up-bridge;
in
{
  options.services.fcm2up-bridge = {
    enable = mkEnableOption "fcm2up-bridge FCM to UnifiedPush relay server";

    package = mkOption {
      type = types.package;
      default = flake.packages.${pkgs.system}.fcm2up-bridge;
      defaultText = literalExpression "flake.packages.\${pkgs.system}.fcm2up-bridge";
      description = "The fcm2up-bridge package to use.";
    };

    port = mkOption {
      type = types.port;
      default = 8080;
      description = "HTTP server port for registration API.";
    };

    stateDir = mkOption {
      type = types.str;
      default = "/var/lib/fcm2up-bridge";
      description = "Directory to store state (SQLite database).";
    };

    user = mkOption {
      type = types.str;
      default = "fcm2up-bridge";
      description = "User to run fcm2up-bridge as.";
    };

    group = mkOption {
      type = types.str;
      default = "fcm2up-bridge";
      description = "Group to run fcm2up-bridge as.";
    };
  };

  config = mkIf cfg.enable {
    users.users.${cfg.user} = {
      inherit (cfg) group;
      isSystemUser = true;
      description = "fcm2up-bridge service user";
      home = cfg.stateDir;
    };

    users.groups.${cfg.group} = { };

    systemd.tmpfiles.rules = [
      "d ${cfg.stateDir} 0750 ${cfg.user} ${cfg.group} -"
    ];

    systemd.services.fcm2up-bridge = {
      description = "FCM to UnifiedPush relay server";
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];

      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        WorkingDirectory = cfg.stateDir;
        Restart = "on-failure";
        RestartSec = "10s";

        # Hardening
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictSUIDSGID = true;
        RestrictNamespaces = true;
        ReadWritePaths = [ cfg.stateDir ];
      };

      environment = {
        PORT = toString cfg.port;
        DB_PATH = "${cfg.stateDir}/fcm2up.db";
        RUST_LOG = "info";
      };

      script = ''
        exec ${cfg.package}/bin/fcm2up-bridge
      '';
    };
  };
}
