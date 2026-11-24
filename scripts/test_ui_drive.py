#!/usr/bin/env python3
"""Tests for the driver launcher's scratch cleanup."""
import importlib.machinery
import importlib.util
import os
import signal
import sys
import tempfile
import unittest
from unittest import mock

loader = importlib.machinery.SourceFileLoader("ui_drive", os.path.join(os.path.dirname(__file__), "ui-drive"))
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_loader(loader.name, loader)
driver = importlib.util.module_from_spec(spec)
loader.exec_module(driver)


class StopTest(unittest.TestCase):
    def test_stop_terminates_group_and_removes_owned_scratch(self):
        with tempfile.TemporaryDirectory(prefix="mast-drive.", dir="/private/tmp") as root:
            with tempfile.TemporaryDirectory(prefix="mast-driver-session.", dir="/private/tmp") as session:
                with open(os.path.join(session, "root"), "w") as file:
                    file.write(root)
                with open(os.path.join(session, "pid"), "w") as file:
                    file.write("4321")
                with mock.patch.object(driver, "SESSION", session), \
                     mock.patch.object(driver, "pid_alive", return_value=True), \
                     mock.patch.object(driver, "call", return_value={}), \
                     mock.patch.object(driver.subprocess, "run", return_value=mock.Mock(stdout="drive_launcher")), \
                     mock.patch.object(driver.os, "getpgid", return_value=4321), \
                     mock.patch.object(driver.os, "killpg", side_effect=[None, ProcessLookupError]) as killpg:
                    driver.stop()
                self.assertEqual(killpg.call_args_list[0], mock.call(4321, signal.SIGTERM))
                self.assertFalse(os.path.exists(root))


if __name__ == "__main__":
    unittest.main()
