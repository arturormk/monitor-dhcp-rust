Name:           monitor-dhcp
Version:        %{package_version}
Release:        1%{?dist}
Summary:        Deterministic DHCPv4 server for a dedicated Ethernet interface
License:        MIT
URL:            https://github.com/arturormk/monitor-dhcp-rust
BuildArch:      x86_64
Requires:       systemd
Requires(pre):  shadow-utils
Source0:        monitor-dhcp
Source1:        monitor-dhcp.service
Source2:        config.toml
Source3:        README.md
Source4:        LICENSE
Source5:        monitor-dhcp@.service
Source6:        monitor-dhcp.8
Source7:        HOWTO-systemd.md

%description
A small DHCPv4 server which gives configured addresses to known MAC
addresses and one deterministic default address to unknown clients.

%prep

%build

%install
install -D -m 0755 %{SOURCE0} %{buildroot}%{_bindir}/monitor-dhcp
install -D -m 0644 %{SOURCE1} %{buildroot}%{_unitdir}/monitor-dhcp.service
install -D -m 0644 %{SOURCE5} %{buildroot}%{_unitdir}/monitor-dhcp@.service
install -D -m 0640 %{SOURCE2} %{buildroot}%{_sysconfdir}/monitor-dhcp/config.toml
install -D -m 0644 %{SOURCE3} %{buildroot}%{_docdir}/monitor-dhcp/README.md
install -D -m 0644 %{SOURCE4} %{buildroot}%{_licensedir}/monitor-dhcp/LICENSE
install -D -m 0644 %{SOURCE6} %{buildroot}%{_mandir}/man8/monitor-dhcp.8
install -D -m 0644 %{SOURCE7} %{buildroot}%{_docdir}/monitor-dhcp/HOWTO-systemd.md

%pre
getent group monitor-dhcp >/dev/null || groupadd --system monitor-dhcp
getent passwd monitor-dhcp >/dev/null || \
    useradd --system --gid monitor-dhcp --no-create-home \
        --shell /sbin/nologin monitor-dhcp

%post
chown root:monitor-dhcp %{_sysconfdir}/monitor-dhcp
chmod 0750 %{_sysconfdir}/monitor-dhcp
chown root:monitor-dhcp %{_sysconfdir}/monitor-dhcp/config.toml
chmod 0640 %{_sysconfdir}/monitor-dhcp/config.toml
systemctl daemon-reload >/dev/null 2>&1 || true

%preun
if [ "$1" -eq 0 ]; then
    systemctl disable --now monitor-dhcp.service >/dev/null 2>&1 || true
    for link in /etc/systemd/system/multi-user.target.wants/monitor-dhcp@*.service; do
        [ -L "$link" ] || continue
        systemctl disable --now "$(basename "$link")" >/dev/null 2>&1 || true
    done
fi

%postun
systemctl daemon-reload >/dev/null 2>&1 || true

%files
%license %{_licensedir}/monitor-dhcp/LICENSE
%doc %{_docdir}/monitor-dhcp/README.md
%doc %{_docdir}/monitor-dhcp/HOWTO-systemd.md
%{_bindir}/monitor-dhcp
%{_unitdir}/monitor-dhcp.service
%{_unitdir}/monitor-dhcp@.service
%{_mandir}/man8/monitor-dhcp.8*
%dir %attr(0750,root,monitor-dhcp) %{_sysconfdir}/monitor-dhcp
%config(noreplace) %attr(0640,root,monitor-dhcp) %{_sysconfdir}/monitor-dhcp/config.toml

%changelog
* Thu Jan 01 1970 monitor-dhcp contributors - %{package_version}-1
- Automated package build
