# Dumb Cat Archive compressor/decompressor

## Motivation

The purpose of DCA format is creating minimalistic format for collecting multiple files into single one.
Specifically, writing compressor or decompressor in any language and environment should be relatively simple.

For example, original motivation arised when I had to download many small files from given website, but due
to security measures (cookies, time limited tokens in path) doing it outside browser wasn't optimal.

It's very likely that can find Javascript libraries for handling any format, but doing it manually was fun and simple exercise:
```js
function DCAInit() {
    return new Blob(['DCA\n']);
}

function DCAAddFile(archive, fname, blobData) {
    return new Blob([archive, fname, '\n', blobData.size.toString(), '\n', blobData, '\n']);
}

// Returns data as DCA blob
async function urls2dca(urls) {
    archive = DCAInit()
    let i = 1;
    for (url of urls) {
        archive = DCAAddFile(archive, extractFilename(url), await url2blob(url));
        console.log(`Added file ${i}/${urls.length} (${url}) to the output.`);
        i++;
    }
    console.log(`Archive ready to download!`);
    return archive;
}
```

## Format

Grammar is very simple:
```
archive: header '\n' file*
header: 'DCA\n'
file: filename '\n' filesize '\n' payload '\n'
filename: <utf8 encoded filename, must not contain / or \n>
filesize: <decimal utf8 payload size in bytes>
payload: <sequence of `filesize` bytes, original file content>
```

## Command line usage

For complete overview run `dca --help`, but following modes should work.

```sh
# compresses files into archive.dca
$ dca -c file1.txt file2.so -o archive.dca

# note that decompression doesn't create directories
$ mkdir output
# decompressing all files in archive into output directory
$ dca -d archive.dca -o output

# prints archive's contents
$ dca -l archive.dca
```

Many conveniencies work too, such as
```sh
# Compression is assumed ...
$ dca *
# ... unless input is single file and with dca suffix
$ dca archive.dca -o output

# Archive suffix can be implied (creates texts.dca)
$ dca *.txt -o texts
```

## Notes

Aside from command line usage, you can also employ it as a library. There are no required runtine dependencies outside std library at this point, though usual logging facilities are enabled by default.

My original Python implementation is available at src/dsa.py for comparison. Rust version is considerably more robust in error handling and performance.

This project was made in a series of practical studies, evaluations and demonstrations of Rust usage, notably to observe if idiomatic Rust code offers sufficient practical advantage over typical highly performant (like C++) or very productive languages (like Python).

You can judge for yourself, especially by looking at early commits, as they reflected much closely the reference Python script.

Personally, it seemed to me that Rust actively encourages writing well structured and designed code - but if one can't spare the time and effort for finding optimal solutions to the (performance/safety/readability/productivity) design tradeoff, there are compelling alternatives.
