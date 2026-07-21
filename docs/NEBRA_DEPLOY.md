# Nebra Pi Deployment Rules

**Device**: Nebra Pi 3 (`pi@192.168.68.75`, SSH alias `hone-nebra`)  
**Architecture**: aarch64  
**Role**: hone-node clock/sensor relay

## Deployment Rule

**Never install new software on Nebra until old software has been stopped and disabled first.**

The Pi 3 has very limited CPU and RAM. A running service — even one doing nothing — consumes enough resources to make SSH unresponsive and block SCP of large binaries (hone-node is ~60–100MB).

### Pre-install checklist

Before copying any binary or enabling any service:

```bash
# 1. Stop and disable everything being replaced
sudo systemctl stop <old-service>
sudo systemctl disable <old-service>

# 2. Confirm no competing services are running
systemctl list-units --state=running | grep -E 'hone|hone'

# 3. Verify SSH is responsive (try a no-op before SCP)
ssh hone-nebra echo "ok"

# 4. Only then SCP the new binary
scp hone-node hone-nebra:/tmp/hone-node-new
```

### Why this matters

During the hone→hone cutover, `hone-node-v2.service` was left running while attempting to SCP the new hone-node binary. The service saturated all CPU and RAM, causing every SSH/SCP attempt to timeout with "Broken pipe". Recovery required physical reboot + SD card surgery (removing autostart symlinks directly from the mounted filesystem).

This rule prevents that class of failure.

## Service name

Current production service: `hone-node.service`  
Previous (removed): `hone-node-v2.service`, `hone-gnss-capture.service`, `hone-testnet.service`

## SD card recovery (last resort)

If SSH is unresponsive and a reboot doesn't help:

1. Power off Nebra
2. Remove SD card, insert into Grouchly (shows as `/dev/sdc`)
3. Mount: `sudo mount /dev/sdc2 /media/ubuntclaw/rootfs`
4. Remove offending autostart symlinks: `sudo rm /media/ubuntclaw/rootfs/etc/systemd/system/multi-user.target.wants/<service>`
5. Unmount: `sudo umount /media/ubuntclaw/bootfs /media/ubuntclaw/rootfs`
6. Reinsert SD card into Nebra, power on
