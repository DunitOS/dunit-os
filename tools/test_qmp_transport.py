"""Exercise QMP stdio framing, partial I/O, errors, EOF and bounded timeouts."""
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

from qemu_test import Qmp, build_qemu_command


PEER = r'''
import json, os, sys
def emit(value):
    data=(json.dumps(value)+'\n').encode()
    os.write(1,data[:3]);sys.stdout.buffer.write(data[3:]);sys.stdout.buffer.flush()
emit({'QMP':{'version':{}}})
for line in sys.stdin:
    message=json.loads(line)
    command=message['execute']
    if command=='quit':
        emit({'return':{}});break
    if command=='bad':
        emit({'error':{'class':'CommandNotFound'}});continue
    if command=='screendump':
        filename=message['arguments']['filename']
        if filename.endswith('rejected.ppm'):
            emit({'error':{'class':'GenericError','desc':'capture rejected'}});continue
        if not filename.endswith('missing.ppm'):
            with open(filename,'wb') as image:
                image.write(b'P6\n1 1\n255\n\x00\x00\x00')
    emit({'event':'TEST'})
    emit({'return':message.get('arguments',{})})
'''


class StdioTests(unittest.TestCase):
    def peer(self, code=PEER):
        process = subprocess.Popen([sys.executable, '-u', '-c', code],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, bufsize=0)
        self.addCleanup(self.stop, process)
        qmp = Qmp(Path('unused.sock'), reader=process.stdout, writer=process.stdin)
        self.addCleanup(qmp.close)
        return qmp

    @staticmethod
    def stop(process):
        if process.poll() is None:
            process.kill()
        process.wait(timeout=5)

    def test_fragmented_greeting_events_large_payload_and_error(self):
        qmp = self.peer()
        qmp.connect(timeout=2)
        payload = {'text': 'тест' * 20000}
        self.assertEqual(qmp.execute('echo', payload, timeout=2), {'return': payload})
        self.assertEqual(qmp.execute('bad'), {'error': {'class': 'CommandNotFound'}})
        qmp.quit()

    def test_closed_peer_is_reported(self):
        qmp = self.peer('pass')
        with self.assertRaisesRegex(RuntimeError, 'transport closed'):
            qmp.connect(timeout=2)

    def test_screendump_creates_parent_and_requires_an_image(self):
        qmp = self.peer()
        qmp.connect(timeout=2)
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'nested' / 'capture.ppm'
            qmp.screendump(path)
            self.assertTrue(path.read_bytes().startswith(b'P6\n1 1\n'))
            with self.assertRaisesRegex(RuntimeError, 'capture rejected'):
                qmp.screendump(path.with_name('rejected.ppm'))
            with self.assertRaisesRegex(RuntimeError, 'produced no image'):
                qmp.screendump(path.with_name('missing.ppm'))

    def test_silent_peer_has_a_deadline(self):
        qmp = self.peer('import time; time.sleep(10)')
        started = time.monotonic()
        with self.assertRaises(TimeoutError):
            qmp.connect(timeout=0.05)
        self.assertLess(time.monotonic() - started, 1)

    def test_async_events_do_not_extend_response_deadline(self):
        qmp = self.peer(PEER.replace("emit({'return':message.get('arguments',{})})", "\n    if command=='flood':\n        while True: emit({'event':'BUSY'})\n    emit({'return':message.get('arguments',{})})"))
        qmp.connect(timeout=2)
        started = time.monotonic()
        with self.assertRaises(TimeoutError):
            qmp.execute('flood', timeout=0.05)
        self.assertLess(time.monotonic() - started, 1)

    def test_command_preserves_unix_default_and_supports_stdio(self):
        args = (Path('test.iso'), False, 'tcg', '512M', Path('test.sock'), Path('serial.log'), [])
        default = build_qemu_command(*args)
        stdio = build_qemu_command(*args, qmp_transport='stdio')
        self.assertEqual(default[default.index('-qmp') + 1], 'unix:test.sock,server,nowait')
        self.assertEqual(stdio[stdio.index('-qmp') + 1], 'stdio')


if __name__ == '__main__':
    unittest.main()
