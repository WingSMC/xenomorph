import { execSync } from 'node:child_process';
import { arch, platform } from 'node:process';

console.log(`Packaging extension for ${platform}-${arch}...`);

const outputFileName = `xenomorph-${platform}-${arch}.vsix`;

execSync(`vsce package --target ${platform}-${arch} --out ${outputFileName}`, {
    stdio: 'inherit',
});

console.log('✓ Extension packaged successfully!');

const shouldPublish = process.argv.includes('--publish');

if (shouldPublish) {
    console.log('Publishing extension to the marketplace...');
    execSync(`vsce publish --packagePath ${outputFileName}`, {
        stdio: 'inherit',
    });
    console.log('✓ Extension published successfully!');
}
