# reagent on this container: tools for its tasks, nftables with port 80
# open, and the service (/opt/reagent/reagent: a static binary with the web
# UI in it; data in /var/lib/reagent).
{ pkgs, ... }:

{
  networking.nftables.enable = true;
  networking.firewall = {
    enable = true;
    allowedTCPPorts = [ 80 ];
  };

  nix.settings.experimental-features = [ "nix-command" "flakes" ];

  environment.systemPackages = with pkgs; [ git gh curl jq ripgrep vim htop tmux apprise gnumake gcc python3 nodejs ];

  users.users.reagent = {
    isSystemUser = true;
    group = "reagent";
    home = "/var/lib/reagent";
    createHome = true;
    shell = pkgs.bashInteractive;
  };
  users.groups.reagent = { };

  systemd.services.reagent = {
    description = "reagent";
    wantedBy = [ "multi-user.target" ];
    after = [ "network-online.target" ];
    wants = [ "network-online.target" ];
    # What its tasks' commands find.
    # The system's packages too (/run/current-system/sw/bin): what's added there reaches tasks.
    path = [ "/run/current-system/sw" ] ++ (with pkgs; [ bashInteractive coreutils findutils gnugrep gnused gawk diffutils git gh nix openssh curl jq ripgrep apprise gnumake gcc python3 nodejs which procps gnutar gzip xz ]);
    environment = {
      REAGENT_DATA = "/var/lib/reagent";
      HOME = "/var/lib/reagent";
    };
    serviceConfig = {
      User = "reagent";
      Group = "reagent";
      ExecStart = "/opt/reagent/reagent up --listen [::]:80";
      AmbientCapabilities = [ "CAP_NET_BIND_SERVICE" ];
      # Stopping pauses the tasks first (SIGINT); the supervisor (its commands
      # and terminals) stays running across restarts.
      KillSignal = "SIGINT";
      KillMode = "process";
      TimeoutStopSec = 90;
      Restart = "on-failure";
      RestartSec = 5;
    };
  };
}
