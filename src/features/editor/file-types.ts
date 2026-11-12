import type {RemoteEntry} from '../files/file-utils';

const binary = /\.(?:7z|zip|gz|tgz|bz2|xz|tar|rar|jar|war|class|exe|dll|so|dylib|bin|dat|db|sqlite3?|png|jpe?g|gif|webp|ico|bmp|avif|heic|pdf|docx?|xlsx?|pptx?|mp[34]|mkv|mov|avi|wav|ogg|flac|woff2?|ttf|otf|p12|pfx)$/i;
export function canEditFile(entry: RemoteEntry) { return !entry.isDirectory && entry.size <= 2 * 1024 * 1024 && !binary.test(entry.name); }
