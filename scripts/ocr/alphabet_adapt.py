"""Build a local-only adaptation set; keep later camera sessions for evaluation."""
import argparse
from pathlib import Path
import random
import numpy as np
from PIL import Image,ImageEnhance,ImageFilter


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--crops',required=True)
    parser.add_argument('--synthetic',required=True)
    parser.add_argument('--camera-synthetic',required=True)
    parser.add_argument('--output',required=True)
    args=parser.parse_args();rng=random.Random(20261019);randomness=np.random.default_rng(20261019)
    real=np.load(args.crops);images=[];labels=[]
    for image,label in zip(real['images'],real['labels']):
        for _ in range(40):
            source=Image.fromarray(image)
            source=source.rotate(rng.uniform(-7,7),resample=Image.Resampling.BILINEAR,fillcolor=int(np.median(image)))
            width,height=rng.randint(30,96),rng.randint(25,96)
            source=source.resize((width,height),Image.Resampling.BILINEAR).resize((96,96),Image.Resampling.BILINEAR)
            invalid=rng.random()<.2
            source=source.filter(ImageFilter.GaussianBlur(rng.uniform(3.5,7) if invalid else rng.uniform(0,1.2)))
            source=ImageEnhance.Brightness(source).enhance(rng.uniform(.85,1.15))
            array=np.asarray(source,dtype=np.float32)+randomness.normal(0,rng.uniform(0,2),size=(96,96))
            images.append(np.clip(array,0,255).astype('uint8'));labels.append(26 if invalid else int(label))
    synthetic=np.load(args.synthetic);camera=np.load(args.camera_synthetic)
    all_images=np.concatenate([np.stack(images),synthetic['train_images'][:2000],camera['train_images'][:2000]])
    all_labels=np.concatenate([np.array(labels,dtype=np.int64),synthetic['train_labels'][:2000],camera['train_labels'][:2000]])
    # Keep font-held-out synthetic validation; captured motion remains separate.
    np.savez_compressed(args.output,train_images=all_images,train_labels=all_labels,validation_images=camera['validation_images'],validation_labels=camera['validation_labels'])
    print({'training':len(all_labels),'realSourceCrops':len(real['labels']),'validation':len(camera['validation_labels'])})

if __name__=='__main__':main()
