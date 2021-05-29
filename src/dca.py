#!/usr/bin/env python3

# Dumb Cat Archive format compresser/decompresser

# Binary schema is super simple:
# Grammar:
# archive: header '\n' file*
# header: 'DCA\n'
# file: filename '\n' filesize '\n' payload '\n'
# filename: <utf8 encoded filename, must not contain / and \n>
# filesize: <decimal utf8 payload size in bytes>
# payload: <sequence of `filesize` bytes, original file content>

from argparse import ArgumentParser
import logging

argParser = ArgumentParser()
argParser.add_argument("files", nargs='*', help='If decompressing, should be ONLY name of the archive. If compressing, should be list of files.')
argParser.add_argument('--output','-o', help='Name of archive while compressing OR output directory while decompressing.')
modes = argParser.add_mutually_exclusive_group()
modes.add_argument('--compress','-c', dest='mode', action='store_const', const='compress', default='unknown')
modes.add_argument('--decompress','-d', dest='mode', action='store_const', const='decompress')
args = argParser.parse_args()

def decompress_archive(inputFName, outputDir):
    with open(inputFName, 'rb') as input:
        assert input.readline() == b'DCA\n'
        while True:
            fname = input.readline()
            if fname == b'':
                return
            fname = fname[:-1].decode()
            fsize = input.readline()
            fsize = int(fsize[:-1])

            logging.debug(f'Unpacking file `{fname}` of size {fsize}b ...')
            with open(outputDir+'/'+fname, 'wb') as output:
                output.write(input.read(fsize))
            assert input.read(1) == b'\n'

def compress_files(fileNames, archiveName):
    with open(archiveName, 'wb') as output:
        output.write(b'DCA\n')
        for fname in fileNames:
            assert '\n' not in fname
            if '/' in fname:
                storedName = fname[fname.rfind('/')+1:]
            else:
                storedName = fname
            with open(fname, 'rb') as fp:
                blob = fp.read()
                output.write(storedName.encode()+b'\n')
                output.write(str(len(blob)).encode()+b'\n')
                output.write(blob+b'\n')

logging.getLogger().setLevel(logging.DEBUG)

if args.mode == 'unknown':
    if len(args.files) == 1 and args.files[0].endswith('.dca'):
        args.mode = 'decompress'
    else:
        args.mode = 'compress'
if args.mode == 'compress':
    if args.output is None:
        if len(args.files) == 1:
            args.output = args.files[0] + '.dca'
        else:
            args.output = 'dca.dca'
    if '.' not in args.output:
        args.output += '.dca'
    compress_files(args.files, args.output)
elif args.mode == 'decompress':
    assert len(args.files) == 1
    decompress_archive(args.files[0], args.output if args.output is not None else '.')
else:
    print(argParser.format_help())
