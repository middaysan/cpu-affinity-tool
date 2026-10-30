"""Windows diagnostic built-app acceptance. No shipping or real-service certification."""
import argparse
import json
import os
import pathlib
import shutil
import sys
import time
import uuid

from live_mcp_fixture import Fixture, assert_diagnostic_exe, sha256
from live_mcp_scenarios import configuration, connect, execution, history, telemetry, upgrade, steam_selection


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=pathlib.Path, required=True)
    parser.add_argument("--workload", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--interactive-launch", action="store_true", help="Also require a real interactive Explorer broker for ordinary managed launch")
    parser.add_argument("--capture-readme", action="store_true", help="Capture the fixed README scenes instead of the acceptance pack")
    args = parser.parse_args()
    if os.name != "nt":
        parser.error("These scenarios require a Windows desktop")
    exe, workload, root = args.exe.resolve(strict=True), args.workload.resolve(strict=True), args.output.resolve()
    if args.capture_readme:
        if args.interactive_launch:
            parser.error('README capture and interactive acceptance are separate modes')
        from readme_screenshots import capture
        capture(exe, workload, root)
        return
    assert_diagnostic_exe(exe)
    root.mkdir(parents=True, exist_ok=False)
    report = {"artifact_sha256": sha256(exe), "artifact_kind": "diagnostic-dev",
              "shipping_certified": False, "cases": [], "cleanup": "pending"}
    fixtures = []

    def save():
        (root / "result.json").write_text(json.dumps(report, indent=2), encoding="utf-8")

    def case(name, action):
        start = time.monotonic()
        record = {"id": name, "status": "running"}
        report["cases"].append(record)
        save()
        try:
            evidence = action()
            record.update(status="pass", evidence=evidence)
            return evidence
        except Exception as error:
            # Do not serialize state, credentials, a process inventory or raw application output.
            record.update(status="fail", failure_type=type(error).__name__)
            raise
        finally:
            record["seconds"] = round(time.monotonic() - start, 3)
            save()

    try:
        fixture = Fixture(exe, root / "current")
        fixtures.append(fixture)
        client = None
        def start_case():
            nonlocal client
            fixture.start()
            assert not fixture.state()["settings"]["cpu_monitoring"], "Synthetic monitoring must start Off"
            assert json.loads(fixture.state_path.read_bytes())["process_monitoring_enabled"] is False
            client = connect(fixture)
            return {"pid": fixture.process.pid, "default_cpu_off": True, "mcp_default_off": True, "listener_pid_verified": True}
        case("R1-diagnostic-start", start_case)
        report["app_version"] = client.tool("get_capabilities")["app_version"]
        stamp = uuid.uuid4().hex[:10]
        primary, helper = root / ("owned-main-" + stamp + ".exe"), root / ("owned-helper-" + stamp + ".exe")
        shutil.copy2(workload, primary)
        shutil.copy2(workload, helper)
        ids = case("C1-configuration-receipts-restart", lambda: configuration(fixture, client, primary, helper))
        first, group, rule, cpu = ids
        if args.interactive_launch:
            managed = root / "owned-managed.exe"
            shutil.copy2(workload, managed)
            case("E1-managed-launch-native-readback", lambda: execution(fixture, client, managed, cpu))
        else:
            report["cases"].append({"id": "E1-managed-launch-native-readback", "status": "not_requested",
                                    "seconds": 0, "reason": "Requires interactive Explorer; run with --interactive-launch"})
        case("T1-extra-shared-cpu-ram", lambda: telemetry(fixture, client, primary, helper, group, first, rule, cpu))
        case("T2-history-csv-off", lambda: history(fixture, client, cpu, helper))
        old = json.loads(fixture.state_path.read_bytes())
        fixture.close()
        for theme, index in [("dark", 2), ("light", 1)]:
            state = dict(old, version=13, theme_index=index, cpu_load_monitoring_enabled=False)
            state.pop("process_control", None)  # This metadata did not exist in schema 13.
            migration = Fixture(exe, root / ("upgrade-" + theme), state)
            fixtures.append(migration)
            migration.start()
            case("U1-upgrade-retry-" + theme, lambda: upgrade(migration))
        steam_fixture = Fixture(exe, root / 'steam-selection')
        fixtures.append(steam_fixture)
        steam_fixture.start()
        steam_client = connect(steam_fixture)
        steam_workload = steam_fixture.root / 'owned-steam-game.exe'
        shutil.copy2(workload, steam_workload)
        case('S1-steam-warning-selection', lambda: steam_selection(steam_fixture, steam_client, steam_workload))
        report["status"] = "pass"
    except Exception:
        report["status"] = "fail"
        raise
    finally:
        failures = []
        for fixture in reversed(fixtures):
            try:
                fixture.cleanup()
            except Exception as error:
                failures.append(type(error).__name__)
        report["cleanup"] = "fail" if failures else "pass"
        report["cleanup_failures"] = failures
        if failures:
            report["status"] = "fail"
        save()
        print(json.dumps({"status": report.get("status"), "cleanup": report["cleanup"],
                          "cases": [{"id": c["id"], "status": c["status"], "seconds": c["seconds"]} for c in report["cases"]]}))
        if failures:
            raise AssertionError("Synthetic fixture cleanup failed")


if __name__ == "__main__":
    main()
