# Published evidence and path anonymization

The checked-in reports preserve historical measurements and acceptance results.
Personal home-directory paths are represented as `/Users/USER` in published
copies. Original reports remain in local, ignored artifact storage.

Recorded executable, shader, helper, and fixture hashes still identify the
original tested inputs. Recorded raw report/log hashes refer to the original
files before path anonymization; they are not hashes of these edited public
copies. Path anonymization does not represent a new test run.
