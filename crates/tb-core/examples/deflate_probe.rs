// Writes gzip/zlib/deflate samples so they can be cross-checked with python's zlib:
//   cargo run -p tb-core --example deflate_probe
//   python3 -c "import zlib;assert zlib.decompress(open('/tmp/tb_sample.gz','rb').read())==open('/tmp/tb_sample.bin','rb').read()"
use std::io::Write;

fn main() {
    let mut data: Vec<u8> = Vec::new();
    for i in 0..2000 {
        data.extend_from_slice(
            format!("line {i}: the quick brown fox jumps over the lazy dog — taz.de\n").as_bytes(),
        );
    }
    data.extend((0..=255u8).cycle().take(5000));
    std::fs::write("/tmp/tb_sample.bin", &data).unwrap();
    std::fs::write("/tmp/tb_sample.gz", tb_core::gzip_deflate(&data)).unwrap();
    std::fs::write("/tmp/tb_sample.zz", tb_core::zlib_deflate(&data)).unwrap();
    std::fs::write("/tmp/tb_sample.deflate", tb_core::deflate(&data)).unwrap();
    let mut f = std::fs::File::create("/tmp/tb_sample.empty.gz").unwrap();
    f.write_all(&tb_core::gzip_deflate(b"")).unwrap();
    println!(
        "raw {} gzip {} zlib {} deflate {}",
        data.len(),
        tb_core::gzip_deflate(&data).len(),
        tb_core::zlib_deflate(&data).len(),
        tb_core::deflate(&data).len()
    );
}
