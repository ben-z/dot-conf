import os
import shutil
import unittest
from pathlib import Path

from testcontainers.core.container import DockerContainer


class TestContainerIntegration(unittest.TestCase):
    def test_cli_runs_inside_container(self):
        if shutil.which('docker') is None:
            self.skipTest('Docker not available')

        project_root = Path(__file__).resolve().parents[1]
        with DockerContainer('python:3.12') as container:
            container.with_volume_mapping(str(project_root), '/code')
            container.with_workdir('/code')
            container.with_command(
                'sh -c "pip install -r requirements.txt >/dev/null && '
                'pip install -e . >/dev/null && '
                'PYTHONPATH=. pytest tests/integration_test.py -q"'
            )
            logs = container.get_logs()
            output = logs.decode('utf-8') if isinstance(logs, bytes) else str(logs)
            self.assertIn('1 passed', output)


if __name__ == '__main__':
    unittest.main()
