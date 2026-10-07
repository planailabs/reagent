-- Commands in the project's nix dev shell: off | auto (a flake.nix there) | on; which shell (devShells.<attr>).
alter table projects add column devshell text not null default 'auto';
alter table projects add column devshell_attr text;
